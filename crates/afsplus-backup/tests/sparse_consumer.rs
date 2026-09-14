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

#[test]
fn captured_afs_file_group_recovers_exact_metadata_with_explicit_unknown_inventory_loss() {
    use afsplus_backup::{allocation, file, inventory};
    use afsplus_vfs::backup::InventoryKnowledge;
    let mut source = volume();
    let id = source.create_file_in_root("source", &[], time()).unwrap();
    source.write_file_at(id, 4096, b"captured", time()).unwrap();
    source.preallocate_file(id, 8192, 4096, time()).unwrap();
    source.truncate_file(id, (1 << 40) + 23, time()).unwrap();
    let preserved = afsplus_core::volume::PreservedMetadata {
        protection: 0xa5a5,
        created: Timespec {
            seconds: -1234,
            nanoseconds: 987654321,
        },
        modified: time(),
        changed: Timespec {
            seconds: -999,
            nanoseconds: 222333444,
        },
    };
    source.restore_object_metadata(id, preserved).unwrap();
    let snapshot = source.snapshot_create(time()).unwrap();
    source.write_file_at(id, 4096, b"LIVE NOW", time()).unwrap();
    let source =
        mount_with_snapshot_limits(source.into_device(), MountOptions::default(), work()).unwrap();
    let (mut source, authority) = BackupService::new(source, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, snapshot).unwrap();
    let limits = file::Limits {
        contents: consumer::Options {
            map: limits(),
            records: records(),
            page_entries: 1,
        },
        inventory: inventory::Limits {
            values: 16,
            value_bytes: 4096,
            page_entries: 1,
            records: records(),
        },
    };
    let ordinal = u64::MAX - 2;
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let exported = file::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: id,
        },
        &mut writer,
        (ordinal, "files/captured"),
        file::Mode::Recovery,
        &mut [0; 512],
        limits,
    )
    .unwrap();
    assert_eq!(exported.next_ordinal, None);
    assert_eq!(
        exported.knowledge.attributes,
        InventoryKnowledge::Uninspected
    );
    assert_eq!(exported.knowledge.security, InventoryKnowledge::Uninspected);
    let wire = writer.finish().unwrap().0;
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
    let (mut dest, authority) = RestoreService::new(backend, 2).unwrap();
    let grant = authority.grant();
    let root = dest.root(&grant).unwrap();
    let restored = dest
        .create_file(&root, "restored", Timespec::default())
        .unwrap();
    let report = file::restore(
        &mut reader,
        &mut dest.client(),
        &afsplus_backup::attachment::Target {
            ordinal,
            path: "files/captured",
            object: &restored,
        },
        &mut [0; 512],
        file::RestoreOptions {
            mode: file::Mode::Recovery,
            allocation: allocation::RestoreOptions {
                limits: limits.contents,
                mode: allocation::Mode::RecoverContents,
                reservation_chunk: 0,
                reservation_bytes: 0,
                readback_entries: 0,
            },
            inventory: limits.inventory,
        },
        Timespec::default(),
    )
    .unwrap();
    assert_eq!(report.next_ordinal, None);
    assert_eq!(
        report.allocation.allocation,
        allocation::Disposition::Discarded {
            ranges: 1,
            bytes: 4096
        }
    );
    assert!(matches!(
        report.opaque,
        file::OpaqueDisposition::Omitted {
            knowledge: afsplus_vfs::backup::MetadataInventory {
                attributes: InventoryKnowledge::Uninspected,
                security: InventoryKnowledge::Uninspected
            },
            transported: None
        }
    ));
    assert!(reader.next_member().unwrap().is_none());
    dest.client().sync(&grant).unwrap();
    let restored_id = dest.stat(&restored).unwrap().object_id;
    let backend = dest.into_backend().into_volume();
    let mut restored =
        mount_with_snapshot_limits(backend.into_device(), MountOptions::default(), work()).unwrap();
    let stat = restored.stat(restored_id).unwrap().unwrap();
    assert_eq!(stat.size_bytes, (1 << 40) + 23);
    assert_eq!(
        (stat.protection, stat.created, stat.modified, stat.changed),
        (
            preserved.protection,
            preserved.created,
            preserved.modified,
            preserved.changed
        )
    );
    let mut bytes = [0; 8];
    restored
        .read_file_at(restored_id, 4096, &mut bytes)
        .unwrap();
    assert_eq!(&bytes, b"captured");
    let allocations = restored.file_allocation_page(restored_id, 0, 64).unwrap();
    assert!(allocations.eof);
    assert_eq!(allocations.ranges.len(), 1);
    assert!(!allocations.ranges[0].unwritten);
}

#[test]
fn captured_directory_file_and_alias_groups_restore_identity_and_final_metadata() {
    use afsplus_backup::{allocation, attachment, file, inventory, namespace};
    use afsplus_vfs::{backup::InventoryKnowledge, restore::RestoreMetadata};
    let mut source = volume();
    let directory = source.create_directory_in_root("docs", time()).unwrap();
    let file_id = source
        .create_file_in_directory(directory, "file", b"captured", time())
        .unwrap();
    source
        .link_file(file_id, afsplus_format::OBJECT_ROOT, "alias", time())
        .unwrap();
    let file_meta = afsplus_core::volume::PreservedMetadata {
        protection: 0x1234,
        created: Timespec {
            seconds: -111,
            nanoseconds: 123,
        },
        modified: time(),
        changed: Timespec {
            seconds: 222,
            nanoseconds: 345,
        },
    };
    let dir_meta = afsplus_core::volume::PreservedMetadata {
        protection: 0x5678,
        created: Timespec {
            seconds: -333,
            nanoseconds: 456,
        },
        modified: Timespec {
            seconds: 444,
            nanoseconds: 567,
        },
        changed: Timespec {
            seconds: -555,
            nanoseconds: 678,
        },
    };
    source.restore_object_metadata(file_id, file_meta).unwrap();
    source.restore_object_metadata(directory, dir_meta).unwrap();
    let snapshot = source.snapshot_create(time()).unwrap();
    source
        .write_file_at(file_id, 0, b"LIVE NOW", time())
        .unwrap();
    source
        .create_file_in_directory(directory, "later", b"not captured", time())
        .unwrap();
    let source =
        mount_with_snapshot_limits(source.into_device(), MountOptions::default(), work()).unwrap();
    let (mut source, authority) = BackupService::new(source, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, snapshot).unwrap();
    let inventory = inventory::Limits {
        values: 16,
        value_bytes: 4096,
        page_entries: 1,
        records: records(),
    };
    let ns_limits = namespace::Limits {
        records: records(),
        inventory,
    };
    let file_limits = file::Limits {
        contents: consumer::Options {
            map: limits(),
            records: records(),
            page_entries: 1,
        },
        inventory,
    };
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let dir = namespace::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: directory,
        },
        &mut writer,
        &namespace::Binding {
            ordinal: 0,
            path: "files/docs",
            entry: namespace::Entry::Directory,
        },
        file::Mode::Recovery,
        &mut [0; 512],
        ns_limits,
    )
    .unwrap();
    assert_eq!(dir.next_ordinal, Some(2));
    let file = file::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: file_id,
        },
        &mut writer,
        (2, "files/docs/file"),
        file::Mode::Recovery,
        &mut [0; 512],
        file_limits,
    )
    .unwrap();
    assert_eq!(file.next_ordinal, Some(5));
    let alias = namespace::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: file_id,
        },
        &mut writer,
        &namespace::Binding {
            ordinal: 5,
            path: "files/alias",
            entry: namespace::Entry::HardLink {
                primary_path: "files/docs/file",
            },
        },
        file::Mode::Recovery,
        &mut [0; 512],
        ns_limits,
    )
    .unwrap();
    assert_eq!(alias.next_ordinal, Some(7));
    let wire = writer.finish().unwrap().0;
    // Independent standard-library reader sees ordinary directory/link semantics.
    let path = std::env::temp_dir().join(format!("afsplus-namespace-{}.tar", std::process::id()));
    let mut open = std::fs::OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let mut output = open.open(&path).unwrap();
    use std::io::Write;
    output.write_all(&wire).unwrap();
    drop(output);
    let checked=std::process::Command::new("python3").args(["-c","import sys,tarfile\nwith tarfile.open(sys.argv[1]) as t:\n d=t.getmember('files/docs');a=t.getmember('files/alias')\n assert d.isdir() and d.mode==0o700\n assert a.islnk() and a.linkname=='files/docs/file'\n assert t.extractfile(a).read()==b'captured'\n"]).arg(&path).status().unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(checked.success());
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
    let mut dest = volume();
    let outside = dest
        .create_file_in_root("outside", b"untouched", time())
        .unwrap();
    let selected = dest.create_directory_in_root("selected", time()).unwrap();
    let (mut dest, authority) =
        RestoreService::new(AfsRestoreDestination::new(dest, selected).unwrap(), 3).unwrap();
    let grant = authority.grant();
    let root = dest.root(&grant).unwrap();
    let directory = dest.create_directory(&root, "docs", time()).unwrap();
    let restored_dir_id = dest.stat(&directory).unwrap().object_id;
    let options = namespace::RestoreOptions {
        mode: file::Mode::Recovery,
        limits: ns_limits,
    };
    let report = namespace::restore_directory(
        &mut reader,
        &mut dest.client(),
        &attachment::Target {
            ordinal: 0,
            path: "files/docs",
            object: &directory,
        },
        &mut [0; 512],
        options,
    )
    .unwrap();
    assert_eq!(report.next_ordinal, Some(2));
    assert!(matches!(
        report.opaque,
        file::OpaqueDisposition::Omitted {
            transported: None,
            ..
        }
    ));
    let restored = dest.create_file(&directory, "file", time()).unwrap();
    drop(directory);
    let report = file::restore(
        &mut reader,
        &mut dest.client(),
        &attachment::Target {
            ordinal: 2,
            path: "files/docs/file",
            object: &restored,
        },
        &mut [0; 512],
        file::RestoreOptions {
            mode: file::Mode::Recovery,
            allocation: allocation::RestoreOptions {
                limits: file_limits.contents,
                mode: allocation::Mode::RecoverContents,
                reservation_chunk: 0,
                reservation_bytes: 0,
                readback_entries: 0,
            },
            inventory,
        },
        time(),
    )
    .unwrap();
    let knowledge = match report.opaque {
        file::OpaqueDisposition::Omitted { knowledge, .. } => knowledge,
        _ => unreachable!(),
    };
    assert_eq!(knowledge.security, InventoryKnowledge::Uninspected);
    let report = namespace::restore_alias(
        &mut reader,
        &mut dest.client(),
        &namespace::AliasTarget {
            ordinal: 5,
            path: "files/alias",
            parent: &root,
            name: "alias",
            primary: namespace::Primary {
                object: &restored,
                path: "files/docs/file",
                knowledge,
            },
        },
        options,
        time(),
    )
    .unwrap();
    assert_eq!(report.links, 2);
    assert_eq!(report.next_ordinal, Some(7));
    let restored_file_id = report.object_id;
    drop(restored);
    let directory = dest.lookup_created(&root, "docs").unwrap();
    dest.metadata(
        &directory,
        RestoreMetadata {
            protection: dir_meta.protection as u64,
            created: dir_meta.created,
            modified: dir_meta.modified,
            changed: dir_meta.changed,
        },
    )
    .unwrap();
    assert!(reader.next_member().unwrap().is_none());
    dest.sync(&grant).unwrap();
    let volume = dest.into_backend().into_volume();
    let mut volume =
        mount_with_snapshot_limits(volume.into_device(), MountOptions::default(), work()).unwrap();
    assert_eq!(volume.read_file(outside).unwrap(), b"untouched");
    assert_eq!(
        volume.lookup_in_directory(selected, "alias").unwrap(),
        Some(restored_file_id)
    );
    assert_eq!(
        volume.lookup_in_directory(restored_dir_id, "file").unwrap(),
        Some(restored_file_id)
    );
    assert_eq!(volume.read_file(restored_file_id).unwrap(), b"captured");
    assert!(volume
        .lookup_in_directory(restored_dir_id, "later")
        .unwrap()
        .is_none());
    for (id, expected) in [(restored_dir_id, dir_meta), (restored_file_id, file_meta)] {
        let stat = volume.stat(id).unwrap().unwrap();
        assert_eq!(
            (stat.protection, stat.created, stat.modified, stat.changed),
            (
                expected.protection,
                expected.created,
                expected.modified,
                expected.changed
            )
        );
    }
    assert_eq!(
        volume.stat(restored_file_id).unwrap().unwrap().link_count,
        2
    );
}

fn alias_only_archive(primary_path: &str) -> Vec<u8> {
    use afsplus_backup::{file, inventory, namespace};
    let mut source = volume();
    let id = source
        .create_file_in_root("primary", b"source", time())
        .unwrap();
    let snapshot = source.snapshot_create(time()).unwrap();
    let (mut source, authority) = BackupService::new(source, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, snapshot).unwrap();
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    namespace::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: id,
        },
        &mut writer,
        &namespace::Binding {
            ordinal: u64::MAX - 1,
            path: "files/alias",
            entry: namespace::Entry::HardLink { primary_path },
        },
        file::Mode::Recovery,
        &mut [0; 1],
        namespace::Limits {
            records: records(),
            inventory: inventory::Limits {
                values: 16,
                value_bytes: 4096,
                page_entries: 1,
                records: records(),
            },
        },
    )
    .unwrap();
    writer.finish().unwrap().0
}
#[test]
fn alias_conflicts_and_resource_refusal_do_not_create_a_link() {
    use afsplus_backup::{file, inventory, namespace};
    use afsplus_vfs::backup::{InventoryKnowledge, MetadataInventory};
    for fault in 0..=8 {
        let wire = alias_only_archive(if fault == 5 {
            "files/wrong"
        } else {
            "files/primary"
        });
        let mut spool = spool::Verified::capture(
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
        let (mut dest, authority) = RestoreService::new(
            AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap(),
            if fault == 0 { 2 } else { 3 },
        )
        .unwrap();
        let grant = authority.grant();
        let root = dest.root(&grant).unwrap();
        let primary = dest.create_file(&root, "primary", time()).unwrap();
        dest.write(&primary, 0, b"restored", time()).unwrap();
        let primary_id = dest.stat(&primary).unwrap().object_id;
        if fault == 1 {
            let occupied = dest.create_file(&root, "alias", time()).unwrap();
            drop(occupied);
        }
        if fault == 2 {
            dest.metadata(
                &primary,
                afsplus_vfs::restore::RestoreMetadata {
                    protection: 1,
                    created: time(),
                    modified: time(),
                    changed: time(),
                },
            )
            .unwrap();
        }
        if fault == 3 {
            authority.revoke(&grant).unwrap();
        }
        let knowledge = MetadataInventory {
            attributes: if fault == 4 {
                InventoryKnowledge::Empty
            } else {
                InventoryKnowledge::Uninspected
            },
            security: InventoryKnowledge::Uninspected,
        };
        let result = namespace::restore_alias(
            &mut reader,
            &mut dest.client(),
            &namespace::AliasTarget {
                ordinal: if fault == 8 { u64::MAX } else { u64::MAX - 1 },
                path: "files/alias",
                parent: &root,
                name: if fault == 6 { "wrong" } else { "alias" },
                primary: namespace::Primary {
                    object: &primary,
                    path: "files/primary",
                    knowledge,
                },
            },
            namespace::RestoreOptions {
                mode: if fault == 7 {
                    file::Mode::Full
                } else {
                    file::Mode::Recovery
                },
                limits: namespace::Limits {
                    records: records(),
                    inventory: inventory::Limits {
                        values: 16,
                        value_bytes: 4096,
                        page_entries: 1,
                        records: records(),
                    },
                },
            },
            time(),
        );
        assert!(result.is_err(), "fault {fault}");
        assert!(reader.next_member().is_err());
        let mut volume = dest.into_backend().into_volume();
        assert_eq!(volume.stat(primary_id).unwrap().unwrap().link_count, 1);
        assert_eq!(volume.lookup_root("alias").unwrap().is_some(), fault == 1);
        assert_eq!(volume.read_file(primary_id).unwrap(), b"restored");
    }
}
#[test]
fn alias_last_ordinal_is_explicit_and_identity_is_shared() {
    use afsplus_backup::{file, inventory, namespace};
    use afsplus_vfs::backup::{InventoryKnowledge, MetadataInventory};
    let wire = alias_only_archive("files/primary");
    let mut spool = spool::Verified::capture(
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
    let (mut dest, authority) = RestoreService::new(
        AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap(),
        3,
    )
    .unwrap();
    let grant = authority.grant();
    let root = dest.root(&grant).unwrap();
    let primary = dest.create_file(&root, "primary", time()).unwrap();
    dest.write(&primary, 0, b"restored", time()).unwrap();
    let report = namespace::restore_alias(
        &mut reader,
        &mut dest.client(),
        &namespace::AliasTarget {
            ordinal: u64::MAX - 1,
            path: "files/alias",
            parent: &root,
            name: "alias",
            primary: namespace::Primary {
                object: &primary,
                path: "files/primary",
                knowledge: MetadataInventory {
                    attributes: InventoryKnowledge::Uninspected,
                    security: InventoryKnowledge::Uninspected,
                },
            },
        },
        namespace::RestoreOptions {
            mode: file::Mode::Recovery,
            limits: namespace::Limits {
                records: records(),
                inventory: inventory::Limits {
                    values: 16,
                    value_bytes: 4096,
                    page_entries: 1,
                    records: records(),
                },
            },
        },
        time(),
    )
    .unwrap();
    assert_eq!(report.next_ordinal, None);
    assert_eq!(report.links, 2);
    assert!(reader.next_member().unwrap().is_none());
    let alias = dest.lookup_created(&root, "alias").unwrap();
    dest.write(&alias, 0, b"changed!", time()).unwrap();
    let mut bytes = [0; 8];
    dest.client().read(&primary, 0, &mut bytes).unwrap();
    assert_eq!(&bytes, b"changed!");
}

fn symlink_namespace_limits() -> afsplus_backup::namespace::Limits {
    let mut records = records();
    records.bytes = 8192;
    records.value_bytes = 4096;
    afsplus_backup::namespace::Limits {
        records,
        inventory: afsplus_backup::inventory::Limits {
            values: 16,
            value_bytes: 4096,
            page_entries: 1,
            records,
        },
    }
}
fn captured_symlink_archive(target: &str) -> Vec<u8> {
    use afsplus_backup::{file, namespace};
    let mut source = volume();
    let id = source
        .create_symlink(afsplus_format::OBJECT_ROOT, "link", target, time())
        .unwrap();
    source.set_object_protection(id, 0x8765, time()).unwrap();
    let snapshot = source.snapshot_create(time()).unwrap();
    source
        .unlink_symlink(afsplus_format::OBJECT_ROOT, "link", time())
        .unwrap();
    let (mut source, authority) = BackupService::new(source, 1).unwrap();
    let grant = authority.grant();
    let view = source.open(&grant, snapshot).unwrap();
    let mut writer = envelope::Writer::new(Vec::new(), framing()).unwrap();
    let report = namespace::export(
        &mut Captured {
            client: &mut source.client(),
            reader: &view,
            object: id,
        },
        &mut writer,
        &namespace::Binding {
            ordinal: 0,
            path: "files/link",
            entry: namespace::Entry::Symlink,
        },
        file::Mode::Recovery,
        &mut [0; 1],
        symlink_namespace_limits(),
    )
    .unwrap();
    assert_eq!(report.next_ordinal, Some(2));
    writer.finish().unwrap().0
}

#[test]
fn captured_symlink_archive_restores_exact_opaque_targets_and_metadata() {
    use afsplus_backup::{file, namespace};
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let maximum = "x".repeat(3968);
    for target in [
        "../missing",
        "/outside//path",
        "SYS:Tools",
        "../café/日本語",
        &maximum,
    ] {
        let wire = captured_symlink_archive(target);
        let mut child = Command::new("python3").args(["-c", "import sys,tarfile,io\nt=tarfile.open(fileobj=io.BytesIO(sys.stdin.buffer.read()))\nx=t.getmember('files/link')\nassert x.issym() and x.linkname==sys.argv[1] and x.size==0\n", target])
            .stdin(Stdio::piped()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(&wire).unwrap();
        assert!(child.wait().unwrap().success());
        let limits = symlink_namespace_limits();
        let mut spool = spool::Verified::capture_sparse(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            spool::Limits {
                chunk_bytes: 512,
                archive_bytes: wire.len() as u64,
                store_bytes: wire.len() as u64 * 2,
            },
            framing(),
            limits.records,
        )
        .unwrap();
        let mut reader = spool.reader(framing(), limits.records).unwrap();
        let mut dest = volume();
        let outside = dest
            .create_file_in_root("outside", b"untouched", time())
            .unwrap();
        let selected = dest.create_directory_in_root("selected", time()).unwrap();
        let (mut dest, authority) =
            RestoreService::new(AfsRestoreDestination::new(dest, selected).unwrap(), 2).unwrap();
        let grant = authority.grant();
        let root = dest.root(&grant).unwrap();
        let report = namespace::restore_symlink(
            &mut reader,
            &mut dest.client(),
            &namespace::SymlinkTarget {
                ordinal: 0,
                path: "files/link",
                parent: &root,
                name: "link",
            },
            &mut [0; 1],
            namespace::RestoreOptions {
                mode: file::Mode::Recovery,
                limits,
            },
            time(),
        )
        .unwrap();
        assert_eq!(report.next_ordinal, Some(2));
        assert!(matches!(
            report.opaque,
            file::OpaqueDisposition::Omitted { .. }
        ));
        let id = dest.stat(&report.object).unwrap().object_id;
        assert!(reader.next_member().unwrap().is_none());
        dest.sync(&grant).unwrap();
        drop(report);
        drop(root);
        let mut volume = mount_with_snapshot_limits(
            dest.into_backend().into_volume().into_device(),
            MountOptions::default(),
            work(),
        )
        .unwrap();
        let mut bytes = vec![0; target.len()];
        assert_eq!(volume.read_link(id, &mut bytes).unwrap(), target.len());
        assert_eq!(bytes, target.as_bytes());
        let stat = volume.stat(id).unwrap().unwrap();
        assert_eq!(stat.protection, 0x8765);
        assert_eq!(stat.created, time());
        assert_eq!(stat.modified, time());
        assert_eq!(stat.changed, time());
        assert_eq!(volume.read_file(outside).unwrap(), b"untouched");
    }
}

#[test]
fn symlink_archive_refusals_do_not_create_destination_entries() {
    use afsplus_backup::{file, namespace};
    let wire = captured_symlink_archive("../opaque");
    for fault in 0..6 {
        let limits = symlink_namespace_limits();
        let mut spool = spool::Verified::capture_sparse(
            wire.as_slice(),
            Cursor::new(Vec::new()),
            spool::Limits {
                chunk_bytes: 512,
                archive_bytes: wire.len() as u64,
                store_bytes: wire.len() as u64 * 2,
            },
            framing(),
            limits.records,
        )
        .unwrap();
        let mut reader = spool.reader(framing(), limits.records).unwrap();
        let (mut dest, authority) = RestoreService::new(
            AfsRestoreDestination::new(volume(), afsplus_format::OBJECT_ROOT).unwrap(),
            if fault == 0 { 1 } else { 2 },
        )
        .unwrap();
        let grant = authority.grant();
        let root = dest.root(&grant).unwrap();
        if fault == 1 {
            authority.revoke(&grant).unwrap();
        }
        let mut options = namespace::RestoreOptions {
            mode: file::Mode::Recovery,
            limits,
        };
        if fault == 2 {
            options.mode = file::Mode::Full;
        }
        let result = namespace::restore_symlink(
            &mut reader,
            &mut dest.client(),
            &namespace::SymlinkTarget {
                ordinal: if fault == 3 { 1 } else { 0 },
                path: if fault == 4 {
                    "files/other"
                } else {
                    "files/link"
                },
                parent: &root,
                name: if fault == 5 { "other" } else { "link" },
            },
            &mut [0; 1],
            options,
            time(),
        );
        assert!(result.is_err(), "fault {fault}");
        assert!(reader.next_member().is_err());
        drop(result);
        drop(root);
        let mut volume = dest.into_backend().into_volume();
        assert_eq!(
            volume
                .lookup_in_directory(afsplus_format::OBJECT_ROOT, "link")
                .unwrap(),
            None
        );
        assert_eq!(
            volume
                .lookup_in_directory(afsplus_format::OBJECT_ROOT, "other")
                .unwrap(),
            None
        );
    }
}
