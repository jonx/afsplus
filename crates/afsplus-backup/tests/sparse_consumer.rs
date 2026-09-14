#![cfg(feature = "consumer")]
use afsplus_backup::{
    attachment::Captured,
    envelope, pax,
    sparse::{self, consumer},
    spool, tar,
};
use afsplus_block::{MemoryBackend, TraceBackend};
use afsplus_core::{mount_with_snapshot_limits, volume::SnapshotWorkLimits, MountOptions, Volume};
use afsplus_format::Timespec;
use afsplus_vfs::{
    backup::BackupService,
    restore::{AfsRestoreDestination, RestoreService},
};
use std::io::Cursor;
fn time() -> Timespec {
    Timespec {
        seconds: -42,
        nanoseconds: 123456789,
    }
}
fn work() -> SnapshotWorkLimits {
    SnapshotWorkLimits {
        max_edit_records: 4096,
        max_views: 8,
        reclaim_records: 1,
    }
}
fn volume() -> Volume<MemoryBackend> {
    let mut dev = MemoryBackend::new(4096, 1024);
    afsplus_core::mkfs_with_options(
        &mut dev,
        &afsplus_core::MkfsParams {
            uuid: [0x97; 16],
            label: "SparseConsumer".into(),
            region_size: 1024,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: false,
            name_policy: afsplus_core::NamePolicy::Sensitive,
            timestamp: time(),
        },
        afsplus_core::MkfsOptions {
            persistent_snapshots: true,
        },
    )
    .unwrap();
    mount_with_snapshot_limits(dev, MountOptions::default(), work()).unwrap()
}
fn framing() -> tar::Limits {
    tar::Limits {
        members: 16,
        member_bytes: 65536,
        trailing_zero_blocks: 0,
    }
}
fn records() -> pax::Limits {
    pax::Limits {
        bytes: 4096,
        records: 16,
        key_bytes: 64,
        value_bytes: 1024,
    }
}
fn limits() -> sparse::Limits {
    sparse::Limits {
        entries: 64,
        map_bytes: 4096,
        data_bytes: 65536,
        logical_bytes: u64::MAX,
    }
}
fn archived() -> (Vec<u8>, consumer::Report) {
    let mut source = volume();
    let file = source.create_file_in_root("source", &[], time()).unwrap();
    source
        .write_file_at(file, 4096, b"captured", time())
        .unwrap();
    source
        .write_file_at(file, 4 * 4096, &[0; 4096], time())
        .unwrap();
    source
        .preallocate_file(file, 2 * 4096, 4096, time())
        .unwrap();
    source
        .preallocate_file(file, (1 << 40) + 4096, 4096, time())
        .unwrap();
    source.truncate_file(file, (1 << 40) + 23, time()).unwrap();
    let id = source.snapshot_create(time()).unwrap();
    let mut source = mount_with_snapshot_limits(
        TraceBackend::new(source.into_device()),
        MountOptions::default(),
        work(),
    )
    .unwrap();
    source
        .write_file_at(file, 4096, b"LIVE NOW", time())
        .unwrap();
    source.truncate_file(file, 8192, time()).unwrap();
    source.device_mut().reset();
    let (mut service, authority) = BackupService::new(source, 1).unwrap();
    let grant = authority.grant();
    let view = service.open(&grant, id).unwrap();
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let report = consumer::export(
        &mut Captured {
            client: &mut service.client(),
            reader: &view,
            object: file,
        },
        &mut writer,
        9,
        "files/sparse",
        &mut [0; 1024],
        consumer::Options {
            map: limits(),
            records: records(),
            page_entries: 1,
        },
    )
    .unwrap();
    let io = service.backend_mut().device_mut().stats();
    assert_eq!(io.writes, 0);
    assert_eq!(io.flushes, 0);
    assert!(io.reads < 256, "hole traversal expanded: {}", io.reads);
    let wire = writer.finish().unwrap().0;
    assert!(wire.len() < 16384);
    (wire, report)
}
#[test]
fn captured_sparse_contents_cross_real_services_and_remount_without_gap_expansion() {
    let (wire, report) = archived();
    assert_eq!(report.logical_bytes, (1 << 40) + 23);
    assert_eq!(report.written_bytes, 8192);
    assert_eq!(report.stored_bytes, 8704);
    assert_eq!(report.omitted_unwritten_ranges, 2);
    assert_eq!(report.omitted_unwritten_bytes, 8192);
    assert!(spool::Verified::capture(
        wire.as_slice(),
        Cursor::new(Vec::new()),
        spool::Limits {
            chunk_bytes: 512,
            archive_bytes: wire.len() as u64,
            store_bytes: wire.len() as u64 * 2
        },
        framing(),
        records()
    )
    .is_err());
    let mut spool = spool::Verified::capture_sparse(
        wire.as_slice(),
        Cursor::new(Vec::new()),
        spool::Limits {
            chunk_bytes: 512,
            archive_bytes: wire.len() as u64,
            store_bytes: wire.len() as u64 * 2,
        },
        framing(),
        records(),
    )
    .unwrap();
    let mut reader = spool.reader(framing(), records()).unwrap();
    let mut destination = volume();
    let outside = destination
        .create_file_in_root("outside", b"untouched", time())
        .unwrap();
    let root = destination
        .create_directory_in_root("destination", time())
        .unwrap();
    let backend = AfsRestoreDestination::new(destination, root).unwrap();
    let (mut service, authority) = RestoreService::new(backend, 2).unwrap();
    let grant = authority.grant();
    let parent = service.root(&grant).unwrap();
    let file = service.create_file(&parent, "restored", time()).unwrap();
    let map = consumer::restore(
        &mut reader,
        &mut service.client(),
        &consumer::Target {
            path: "files/sparse",
            object: &file,
        },
        &mut [0; 4096],
        limits(),
        time(),
    )
    .unwrap();
    assert_eq!(map.data_bytes(), 8192);
    assert!(reader.next_member().unwrap().is_none());
    service.client().sync(&grant).unwrap();
    let id = service.client().stat(&file).unwrap().object_id;
    drop(file);
    drop(parent);
    let volume = service.into_backend().into_volume();
    let mut volume =
        mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), work()).unwrap();
    assert_eq!(volume.read_file(outside).unwrap(), b"untouched");
    let stat = volume.stat(id).unwrap().unwrap();
    assert_eq!(stat.size_bytes, (1 << 40) + 23);
    assert_eq!(stat.allocated_bytes, 8192);
    for offset in [0, 8192, 4 * 4096, 1 << 39, 1 << 40] {
        let mut zero = [99; 23];
        assert_eq!(volume.read_file_at(id, offset, &mut zero).unwrap(), 23);
        assert_eq!(zero, [0; 23]);
    }
    let mut content = [0; 8];
    volume.read_file_at(id, 4096, &mut content).unwrap();
    assert_eq!(&content, b"captured");
    let view_id = volume.snapshot_create(time()).unwrap();
    let view = volume.snapshot_open(view_id).unwrap();
    let allocations = volume.snapshot_allocation_page(&view, id, 0, 64).unwrap();
    assert_eq!(allocations.ranges.len(), 2);
    assert!(allocations.ranges.iter().all(|r| !r.unwritten));
}
#[test]
fn wrong_binding_revocation_and_nonempty_destination_refuse_and_poison() {
    let (wire, _) = archived();
    for fault in 0..4 {
        let mut spool = spool::Verified::capture_sparse(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            spool::Limits {
                chunk_bytes: 512,
                archive_bytes: wire.len() as u64,
                store_bytes: wire.len() as u64 * 2,
            },
            framing(),
            records(),
        )
        .unwrap();
        let mut reader = spool.reader(framing(), records()).unwrap();
        let backend = AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap();
        let (mut service, authority) = RestoreService::new(backend, 2).unwrap();
        let grant = authority.grant();
        let parent = service.root(&grant).unwrap();
        let file = service.create_file(&parent, "restored", time()).unwrap();
        if fault == 1 {
            authority.revoke(&grant).unwrap();
        }
        if fault == 2 {
            service.write(&file, 0, b"existing", time()).unwrap();
        }
        let mut bound = limits();
        if fault == 3 {
            bound.entries = 0;
        }
        assert!(consumer::restore(
            &mut reader,
            &mut service.client(),
            &consumer::Target {
                path: if fault == 0 {
                    "files/wrong"
                } else {
                    "files/sparse"
                },
                object: &file,
            },
            &mut [0; 4096],
            bound,
            time()
        )
        .is_err());
        assert!(reader.next_member().is_err());
        if fault != 1 {
            assert_eq!(
                service.client().stat(&file).unwrap().size,
                if fault == 2 { 8 } else { 0 }
            );
        }
    }
}

#[test]
fn source_revocation_during_output_withholds_sparse_completion_even_for_all_holes() {
    use afsplus_vfs::backup::{BackupAuthority, BackupGrant};
    use std::io::{self, Write};
    struct Output {
        bytes: usize,
        authority: BackupAuthority,
        grant: BackupGrant,
        revoked: bool,
    }
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.bytes += bytes.len();
            if self.bytes >= 2048 && !self.revoked {
                self.authority.revoke(&self.grant).unwrap();
                self.revoked = true;
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    for data in [false, true] {
        let mut volume = volume();
        let file = volume
            .create_file_in_root("file", if data { b"data" } else { b"" }, time())
            .unwrap();
        volume.truncate_file(file, 1 << 40, time()).unwrap();
        let id = volume.snapshot_create(time()).unwrap();
        let (mut service, authority) = BackupService::new(volume, 1).unwrap();
        let grant = authority.grant();
        let view = service.open(&grant, id).unwrap();
        let mut writer = envelope::Writer::new(
            Output {
                bytes: 0,
                authority,
                grant,
                revoked: false,
            },
            framing(),
        )
        .unwrap();
        assert!(consumer::export(
            &mut Captured {
                client: &mut service.client(),
                reader: &view,
                object: file
            },
            &mut writer,
            0,
            "files/source",
            &mut [0; 4096],
            consumer::Options {
                map: limits(),
                records: records(),
                page_entries: 1
            }
        )
        .is_err());
        assert!(writer.finish().is_err());
    }
}
