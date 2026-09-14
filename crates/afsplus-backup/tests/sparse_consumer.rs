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

fn allocation_archive(
    size: u64,
    ordinal: u64,
) -> (Vec<u8>, afsplus_backup::allocation::ExportReport) {
    use afsplus_backup::allocation;
    let mut source = volume();
    let file = source.create_file_in_root("source", &[], time()).unwrap();
    if size != 0 {
        source
            .write_file_at(file, 4096, b"captured", time())
            .unwrap();
        source
            .write_file_at(file, 4 * 4096, &[0; 4096], time())
            .unwrap();
    }
    source.preallocate_file(file, 8192, 4096, time()).unwrap();
    source
        .preallocate_file(file, (1 << 40) + 4096, 8192, time())
        .unwrap();
    source
        .preallocate_file(file, u64::MAX - 4095, 4095, time())
        .unwrap();
    source.truncate_file(file, size, time()).unwrap();
    let id = source.snapshot_create(time()).unwrap();
    source
        .write_file_at(file, 4096, b"LIVE NOW", time())
        .unwrap();
    source.truncate_file(file, 8192, time()).unwrap();
    let source =
        mount_with_snapshot_limits(source.into_device(), MountOptions::default(), work()).unwrap();
    let (mut service, authority) = BackupService::new(source, 1).unwrap();
    let grant = authority.grant();
    let view = service.open(&grant, id).unwrap();
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let report = allocation::export(
        &mut Captured {
            client: &mut service.client(),
            reader: &view,
            object: file,
        },
        &mut writer,
        ordinal,
        "files/preserved",
        &mut [0; 4096],
        consumer::Options {
            map: limits(),
            records: records(),
            page_entries: 1,
        },
    )
    .unwrap();
    let wire = writer.finish().unwrap().0;
    assert!(wire.len() < 16384);
    (wire, report)
}
#[test]
fn allocation_preservation_and_explicit_recovery_cross_services_and_remount() {
    use afsplus_backup::allocation::{self, Disposition, Mode};
    use std::collections::BTreeMap;
    for size in [0, (1 << 40) + 23, u64::MAX] {
        let ordinal = u64::MAX - 1;
        let (wire, exported) = allocation_archive(size, ordinal);
        assert_eq!(exported.next_ordinal, None);
        assert_eq!(exported.contents.omitted_unwritten_ranges, 3);
        assert_eq!(exported.contents.omitted_unwritten_bytes, 16384);
        for preserve in [false, true] {
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
            let backend =
                AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap();
            let (mut service, authority) = RestoreService::new(backend, 2).unwrap();
            service.set_reservation_limit(4096);
            let grant = authority.grant();
            let root = service.root(&grant).unwrap();
            let file = service.create_file(&root, "restored", time()).unwrap();
            let report = allocation::restore(
                &mut reader,
                &mut service.client(),
                &allocation::Target {
                    ordinal,
                    path: "files/preserved",
                    object: &file,
                },
                &mut [0; 4096],
                allocation::RestoreOptions {
                    limits: consumer::Options {
                        map: limits(),
                        records: records(),
                        page_entries: 1,
                    },
                    mode: if preserve {
                        Mode::PreserveAllocation
                    } else {
                        Mode::RecoverContents
                    },
                    reservation_chunk: 4096,
                    reservation_bytes: 16384,
                    readback_entries: 64,
                },
                time(),
            )
            .unwrap();
            assert_eq!(report.next_ordinal, None);
            assert_eq!(report.logical_bytes, size);
            assert_eq!(report.written_bytes, if size == 0 { 0 } else { 8192 });
            assert_eq!(
                report.allocation,
                if preserve {
                    Disposition::Preserved
                } else {
                    Disposition::Discarded {
                        ranges: 3,
                        bytes: 16384,
                    }
                }
            );
            assert!(reader.next_member().unwrap().is_none());
            service.client().sync(&grant).unwrap();
            let id = service.stat(&file).unwrap().object_id;
            let volume = service.into_backend().into_volume();
            let mut volume =
                mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), work())
                    .unwrap();
            assert_eq!(volume.stat(id).unwrap().unwrap().size_bytes, size);
            let mut expected = BTreeMap::new();
            if size != 0 {
                expected.insert(1, false);
                expected.insert(4, false);
            }
            if preserve {
                expected.insert(2, true);
                expected.insert((1u64 << 40) / 4096 + 1, true);
                expected.insert((1u64 << 40) / 4096 + 2, true);
                expected.insert(u64::MAX / 4096, true);
            }
            let mut actual = BTreeMap::new();
            let mut cursor = 0;
            loop {
                let page = volume.file_allocation_page(id, cursor, 1).unwrap();
                for r in page.ranges {
                    assert_eq!(r.offset % 4096, 0);
                    assert_eq!(r.length % 4096, 0);
                    for i in 0..r.length / 4096 {
                        assert!(actual.insert(r.offset / 4096 + i, r.unwritten).is_none());
                    }
                }
                if page.eof {
                    break;
                }
                cursor = page.next;
            }
            assert_eq!(actual, expected);
            if size != 0 {
                let mut content = [0; 8];
                volume.read_file_at(id, 4096, &mut content).unwrap();
                assert_eq!(&content, b"captured");
                for offset in [0, 8192, 4 * 4096, 1 << 39, size - 20] {
                    let mut zero = [99; 20];
                    assert_eq!(volume.read_file_at(id, offset, &mut zero).unwrap(), 20);
                    assert_eq!(zero, [0; 20]);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum AllocationProvider {
    Exact,
    UnsupportedReadback,
    UnsupportedReservation,
    ShiftReservation,
    SplitReadback,
}
struct Destination {
    inner: AfsRestoreDestination<MemoryBackend>,
    behavior: AllocationProvider,
    writes: usize,
    reservations: usize,
}
impl afsplus_vfs::restore::RestoreBackend for Destination {
    type Object = u64;
    fn root(&mut self) -> Result<u64, afsplus_vfs::VfsError> {
        self.inner.root()
    }
    fn create_file(&mut self, p: &u64, n: &str, t: Timespec) -> Result<u64, afsplus_vfs::VfsError> {
        self.inner.create_file(p, n, t)
    }
    fn create_directory(
        &mut self,
        p: &u64,
        n: &str,
        t: Timespec,
    ) -> Result<u64, afsplus_vfs::VfsError> {
        self.inner.create_directory(p, n, t)
    }
    fn write(
        &mut self,
        o: &u64,
        offset: u64,
        bytes: &[u8],
        t: Timespec,
    ) -> Result<(), afsplus_vfs::VfsError> {
        self.writes += 1;
        self.inner.write(o, offset, bytes, t)
    }
    fn reserve(
        &mut self,
        o: &u64,
        offset: u64,
        length: u64,
        t: Timespec,
    ) -> Result<(), afsplus_vfs::VfsError> {
        self.reservations += 1;
        if matches!(self.behavior, AllocationProvider::UnsupportedReservation) {
            return Err(afsplus_vfs::VfsError::NotSupported);
        }
        self.inner.reserve(
            o,
            if matches!(self.behavior, AllocationProvider::ShiftReservation) {
                offset + 4096
            } else {
                offset
            },
            length,
            t,
        )
    }
    fn resize(&mut self, o: &u64, size: u64, t: Timespec) -> Result<(), afsplus_vfs::VfsError> {
        self.inner.resize(o, size, t)
    }
    fn link(
        &mut self,
        o: &u64,
        p: &u64,
        n: &str,
        t: Timespec,
    ) -> Result<(), afsplus_vfs::VfsError> {
        self.inner.link(o, p, n, t)
    }
    fn metadata(
        &mut self,
        o: &u64,
        m: afsplus_vfs::restore::RestoreMetadata,
    ) -> Result<(), afsplus_vfs::VfsError> {
        self.inner.metadata(o, m)
    }
    fn stat(&mut self, o: &u64) -> Result<afsplus_vfs::Stat, afsplus_vfs::VfsError> {
        self.inner.stat(o)
    }
    fn read(&mut self, o: &u64, offset: u64, b: &mut [u8]) -> Result<usize, afsplus_vfs::VfsError> {
        self.inner.read(o, offset, b)
    }
    fn sync(&mut self) -> Result<(), afsplus_vfs::VfsError> {
        self.inner.sync()
    }
    fn allocations(
        &mut self,
        o: &u64,
        start: u64,
        limit: usize,
    ) -> Result<afsplus_vfs::backup::AllocationPage, afsplus_vfs::VfsError> {
        use afsplus_vfs::backup::{AllocationPage, AllocationRange};
        if matches!(self.behavior, AllocationProvider::UnsupportedReadback) {
            return Err(afsplus_vfs::VfsError::NotSupported);
        }
        if !matches!(self.behavior, AllocationProvider::SplitReadback) {
            return self.inner.allocations(o, start, limit);
        }
        // Test provider: split the tiny fixture's actual extents into two byte spans.
        let page = self.inner.allocations(o, 0, 64)?;
        assert!(page.eof);
        let ranges: Vec<_> = page
            .ranges
            .into_iter()
            .flat_map(|r| {
                [
                    AllocationRange {
                        length: r.length / 2,
                        ..r
                    },
                    AllocationRange {
                        offset: r.offset + r.length / 2,
                        length: r.length - r.length / 2,
                        ..r
                    },
                ]
            })
            .collect();
        let selected: Vec<_> = ranges
            .iter()
            .skip(start as usize)
            .take(limit)
            .copied()
            .collect();
        let next = start + selected.len() as u64;
        Ok(AllocationPage {
            ranges: selected,
            next,
            eof: next >= ranges.len() as u64,
        })
    }
}
// A separately constructed archive exercises the consumer's semantic binding,
// including a rounded written tail beyond EOF, without sharing its source planner.
fn allocation_fixture(fault: u8) -> Vec<u8> {
    use afsplus_backup::{allocation, member::Timestamp};
    use afsplus_vfs::backup::AllocationRange;
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let layout = allocation::encode(
        if fault == 1 {
            "files/wrong"
        } else {
            "files/preserved"
        },
        if fault == 2 { 4101 } else { 4100 },
        &[
            AllocationRange {
                offset: 4096,
                length: 4096,
                unwritten: fault == 3,
            },
            AllocationRange {
                offset: 16384,
                length: 4096,
                unwritten: true,
            },
        ],
        consumer::Options {
            map: limits(),
            records: records(),
            page_entries: 1,
        },
    )
    .unwrap();
    writer
        .start(
            &tar::Header {
                path: "_AROS_BACKUP/metadata/allocation-0.pax".into(),
                link: String::new(),
                kind: tar::Kind::File,
                mode: 0o600,
                uid: 0,
                gid: 0,
                size: layout.len() as u64,
                mtime: 0,
                uname: String::new(),
                gname: String::new(),
            },
            None,
        )
        .unwrap();
    writer.write_payload(&layout).unwrap();
    let map = sparse::Map::new(
        4100,
        &[sparse::Range {
            offset: if fault == 4 { 4095 } else { 4096 },
            length: 4,
        }],
        limits(),
    )
    .unwrap();
    sparse::start(
        &mut writer,
        &sparse::Entry {
            ordinal: if fault == 5 { 2 } else { 1 },
            path: "files/preserved",
            modified: Timestamp {
                seconds: 0,
                nanos: 0,
            },
        },
        &map,
        records(),
    )
    .unwrap();
    writer.write_payload(b"test").unwrap();
    writer.finish().unwrap().0
}
#[test]
fn allocation_conflicts_and_resource_admission_fail_before_destination_writes() {
    use afsplus_backup::allocation::{self, Mode};
    for fault in 1..=11 {
        let wire = allocation_fixture(if fault <= 5 { fault } else { 0 });
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
        let backend = Destination {
            inner: AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap(),
            behavior: AllocationProvider::Exact,
            writes: 0,
            reservations: 0,
        };
        let (mut service, authority) = RestoreService::new(backend, 2).unwrap();
        service.set_reservation_limit(4096);
        let grant = authority.grant();
        let root = service.root(&grant).unwrap();
        let file = service.create_file(&root, "restored", time()).unwrap();
        if fault == 10 {
            authority.revoke(&grant).unwrap();
        }
        let result = allocation::restore(
            &mut reader,
            &mut service.client(),
            &allocation::Target {
                ordinal: if fault == 11 { u64::MAX } else { 0 },
                path: "files/preserved",
                object: &file,
            },
            &mut [0; 4096],
            allocation::RestoreOptions {
                limits: consumer::Options {
                    map: limits(),
                    records: records(),
                    page_entries: if fault == 9 { 0 } else { 1 },
                },
                mode: Mode::PreserveAllocation,
                reservation_chunk: if fault == 6 { 0 } else { 4096 },
                reservation_bytes: if fault == 7 { 4095 } else { 4096 },
                readback_entries: if fault == 8 { 0 } else { 64 },
            },
            time(),
        );
        assert!(result.is_err(), "fault {fault}");
        assert!(reader.next_member().is_err());
        let backend = service.into_backend();
        assert_eq!(
            (backend.writes, backend.reservations),
            (0, 0),
            "fault {fault}"
        );
    }
}
#[test]
fn allocation_restore_refuses_unsupported_or_wrong_coverage_and_accepts_segmentation() {
    use afsplus_backup::allocation::{self, Disposition, Mode};
    for behavior in [
        AllocationProvider::Exact,
        AllocationProvider::UnsupportedReadback,
        AllocationProvider::UnsupportedReservation,
        AllocationProvider::ShiftReservation,
        AllocationProvider::SplitReadback,
    ] {
        for (preserve, readback_entries) in [(false, 64), (true, 64), (true, 1)] {
            let wire = allocation_fixture(0);
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
            let backend = Destination {
                inner: AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap(),
                behavior,
                writes: 0,
                reservations: 0,
            };
            let (mut service, authority) = RestoreService::new(backend, 2).unwrap();
            service.set_reservation_limit(4096);
            let grant = authority.grant();
            let root = service.root(&grant).unwrap();
            let file = service.create_file(&root, "restored", time()).unwrap();
            let result = allocation::restore(
                &mut reader,
                &mut service.client(),
                &allocation::Target {
                    ordinal: 0,
                    path: "files/preserved",
                    object: &file,
                },
                &mut [0; 4096],
                allocation::RestoreOptions {
                    limits: consumer::Options {
                        map: limits(),
                        records: records(),
                        page_entries: 1,
                    },
                    mode: if preserve {
                        Mode::PreserveAllocation
                    } else {
                        Mode::RecoverContents
                    },
                    reservation_chunk: 4096,
                    reservation_bytes: 4096,
                    readback_entries,
                },
                time(),
            );
            let success = !preserve
                || readback_entries >= 4
                    && matches!(
                        behavior,
                        AllocationProvider::Exact | AllocationProvider::SplitReadback
                    );
            assert_eq!(result.is_ok(), success, "{result:?}");
            if success {
                assert_eq!(
                    result.unwrap().allocation,
                    if preserve {
                        Disposition::Preserved
                    } else {
                        Disposition::Discarded {
                            ranges: 1,
                            bytes: 4096,
                        }
                    }
                );
                let mut bytes = [0; 4];
                service.client().read(&file, 4096, &mut bytes).unwrap();
                assert_eq!(&bytes, b"test");
                assert!(reader.next_member().unwrap().is_none());
            } else {
                assert!(reader.next_member().is_err());
            }
            let backend = service.into_backend();
            if !preserve {
                assert_eq!(backend.reservations, 0);
            }
            if preserve
                && matches!(
                    behavior,
                    AllocationProvider::UnsupportedReadback
                        | AllocationProvider::UnsupportedReservation
                )
            {
                assert_eq!(backend.writes, 0);
            }
            if preserve && matches!(behavior, AllocationProvider::ShiftReservation) {
                assert_eq!(backend.writes, 1);
            }
        }
    }
}
