//! Read retained writer bytes, rather than formatting with the reader under test.
use std::cell::Cell;
use std::rc::Rc;

use afsplus_block::{BlockDevice, BlockError, MemoryBackend, TraceBackend};
use afsplus_check::check_device;
use afsplus_core::{mount, mount_with_options, MountMode, MountOptions, Volume};
use afsplus_format::header::{block_type, BlockHeader};
use afsplus_format::ident::Identification;
use afsplus_format::{Timespec, OBJECT_ROOT};
use sha2::{Digest, Sha256};

const BS: usize = 4096;
const CLEAN: &[u8] = include_bytes!("fixtures/deployed-epoch1/clean.img");
const PENDING: &[u8] = include_bytes!("fixtures/deployed-epoch1/pending.img");
const SYS_HEAD: &[u8] = include_bytes!("fixtures/deployed-epoch1/m1-sys-head.img");

fn time() -> Timespec {
    Timespec {
        seconds: 1_790_770_000,
        nanoseconds: 0,
    }
}

fn image(bytes: &[u8], blocks: u64) -> MemoryBackend {
    assert_eq!(bytes.len() % BS, 0);
    assert!(bytes.len() as u64 <= blocks * BS as u64);
    let mut device = MemoryBackend::new(BS, blocks);
    for (lba, block) in bytes.chunks_exact(BS).enumerate() {
        if block.iter().any(|byte| *byte != 0) {
            device.apply_raw(lba as u64, block);
        }
    }
    device
}

fn baseline_contents<D: BlockDevice>(volume: &mut Volume<D>) {
    assert_eq!(volume.volume_label(), "DeployedBaseline");
    let directory = volume.lookup_root("SYSTEM").unwrap().unwrap();
    let (file, spelling) = volume
        .lookup_entry_in_directory(directory, "readme")
        .unwrap()
        .unwrap();
    assert_eq!(spelling, b"ReadMe");
    assert_eq!(
        volume.read_file(file).unwrap(),
        b"AFS+ deployed image baseline\n"
    );
    assert_eq!(
        volume.object_comment(file).unwrap(),
        "Preserve this comment"
    );
    let metadata = volume.stat(file).unwrap().unwrap();
    assert_eq!(metadata.protection, 0x40);
    assert_eq!(metadata.modified.seconds, 1_790_769_602);
    let payload = volume.lookup_root("payload").unwrap().unwrap();
    assert_eq!(volume.read_file(payload).unwrap(), vec![0xa5; 9000]);
}

fn checked<D: BlockDevice>(mut device: D) {
    let report = check_device(&mut device);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.warnings.is_empty(), "{:?}", report.warnings);
}

#[test]
fn retained_bytes_and_wire_identity_are_fixed() {
    for (bytes, digest, blocks) in [
        (
            CLEAN,
            "fa10bb11224ac50a85dd6e705f82810f4559c5c3282084d10629300febda9813",
            1024,
        ),
        (
            PENDING,
            "1d2fef4c35c9a9d7716efd50b2f61477ab97811570be0ef7413f5799653bafa8",
            1024,
        ),
        (
            SYS_HEAD,
            "863c30061fc2c659710b5e0d89b55c8020f0c6d81ec0bb9e7428a4843fee5cf6",
            65536,
        ),
    ] {
        let actual: String = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        assert_eq!(actual, digest);
        // Literal offsets/values are independent of the production encoder.
        assert_eq!(&bytes[32..40], b"AFSPLUS1");
        assert_eq!(&bytes[4..6], &1u16.to_le_bytes());
        assert_eq!(&bytes[40..44], &1u32.to_le_bytes());
        assert_eq!(&bytes[44..48], &3u32.to_le_bytes());
        assert_eq!(bytes[64], 12);
        let ident = Identification::decode(&bytes[..BS]).unwrap();
        assert_eq!(ident.total_blocks, blocks);
        assert_eq!(
            (
                ident.features.compat,
                ident.features.ro_compat,
                ident.features.incompat
            ),
            (1, 3, 3)
        );
        assert_eq!(ident.unicode_version, [16, 0, 0]);
        for slot in ident.checkpoint_slots {
            let start = usize::try_from(slot).unwrap() * BS;
            let checkpoint = &bytes[start..start + BS];
            if checkpoint[..4] != [0; 4] {
                assert_eq!(&checkpoint[4..6], &1u16.to_le_bytes());
                assert_eq!(&checkpoint[24..28], &168u32.to_le_bytes());
            }
        }
    }
}

#[test]
fn protected_clean_image_is_readable_and_writable_without_conversion() {
    let mut volume = mount_with_options(
        TraceBackend::new(image(CLEAN, 1024)),
        MountOptions {
            mode: MountMode::NoChanges,
            ..Default::default()
        },
    )
    .unwrap();
    baseline_contents(&mut volume);
    let traced = volume.into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);

    let mut volume = mount(traced.into_inner()).unwrap();
    let file = volume.create_file_in_root("New", b"old", time()).unwrap();
    volume
        .write_file_at(file, 0, b"new content", time())
        .unwrap();
    volume
        .rename(OBJECT_ROOT, "New", OBJECT_ROOT, "Renamed", time())
        .unwrap();
    volume
        .create_file_in_root("Temporary", b"remove me", time())
        .unwrap();
    volume.delete_file_in_root("Temporary", time()).unwrap();
    let mut volume = mount(volume.into_device()).unwrap();
    baseline_contents(&mut volume);
    assert_eq!(volume.lookup_root("Renamed").unwrap(), Some(file));
    assert_eq!(volume.read_file(file).unwrap(), b"new content");
    assert_eq!(volume.lookup_root("New").unwrap(), None);
    assert_eq!(volume.lookup_root("Temporary").unwrap(), None);
    checked(volume.into_device());
}

#[test]
fn retained_fsynced_log_replays_with_exact_content() {
    let mut unchanged = mount_with_options(
        TraceBackend::new(image(PENDING, 1024)),
        MountOptions {
            mode: MountMode::NoChanges,
            ..Default::default()
        },
    )
    .unwrap();
    baseline_contents(&mut unchanged);
    assert_eq!(unchanged.pending_intent_records(), 1);
    assert_eq!(unchanged.lookup_root("DurablePending").unwrap(), None);
    let traced = unchanged.into_device();
    assert_eq!(traced.stats().writes, 0);
    assert_eq!(traced.stats().flushes, 0);
    let mut recovered = mount(traced.into_inner()).unwrap();
    baseline_contents(&mut recovered);
    let file = recovered.lookup_root("durablepending").unwrap().unwrap();
    assert_eq!(
        recovered.read_file(file).unwrap(),
        b"fsync survived without a checkpoint\n"
    );
    let mut remounted = mount(recovered.into_device()).unwrap();
    assert_eq!(remounted.lookup_root("DurablePending").unwrap(), Some(file));
    assert_eq!(
        remounted.read_file(file).unwrap(),
        b"fsync survived without a checkpoint\n"
    );
    checked(remounted.into_device());
}

#[test]
fn actual_m1_sys_initializer_mounts_and_preserves_writes() {
    // FSQualify HEAD= writes these exact bytes followed by zeros to 256 MiB.
    let mut volume = mount(image(SYS_HEAD, 65536)).unwrap();
    assert_eq!(volume.volume_label(), "System");
    let file = volume
        .create_file_in_root("BootMarker", b"SYS compatibility\n", time())
        .unwrap();
    let mut volume = mount(volume.into_device()).unwrap();
    assert_eq!(volume.lookup_root("bootmarker").unwrap(), Some(file));
    assert_eq!(volume.read_file(file).unwrap(), b"SYS compatibility\n");
    assert_eq!(volume.volume_label(), "System");
    checked(volume.into_device());
}

struct CountWrites {
    device: MemoryBackend,
    writes: Rc<Cell<u64>>,
}

impl BlockDevice for CountWrites {
    fn block_size(&self) -> usize {
        BS
    }
    fn total_blocks(&self) -> u64 {
        self.device.total_blocks()
    }
    fn read_block(&mut self, lba: u64, data: &mut [u8]) -> Result<(), BlockError> {
        self.device.read_block(lba, data)
    }
    fn write_block(&mut self, lba: u64, data: &[u8]) -> Result<(), BlockError> {
        self.writes.set(self.writes.get() + 1);
        self.device.write_block(lba, data)
    }
    fn flush(&mut self) -> Result<(), BlockError> {
        self.writes.set(self.writes.get() + 1);
        self.device.flush()
    }
}

#[test]
fn unsupported_epoch_and_features_are_refused_without_writes() {
    for epoch in [true, false] {
        let mut device = image(PENDING, 1024);
        let mut block = device.peek(0);
        let header = BlockHeader::verify(&block, block_type::IDENTIFICATION).unwrap();
        if epoch {
            block[40..44].copy_from_slice(&2u32.to_le_bytes());
        } else {
            block[185..193].copy_from_slice(&(3u64 | (1 << 63)).to_le_bytes());
        }
        header.seal(&mut block);
        device.apply_raw(0, &block);
        let writes = Rc::new(Cell::new(0));
        assert!(mount(CountWrites {
            device,
            writes: Rc::clone(&writes)
        })
        .is_err());
        assert_eq!(writes.get(), 0, "refusal must precede pending-log replay");
    }
}

#[test]
fn baseline_oracle_detects_a_semantic_regression() {
    let mut volume = mount(image(CLEAN, 1024)).unwrap();
    let directory = volume.lookup_root("System").unwrap().unwrap();
    let file = volume
        .lookup_in_directory(directory, "ReadMe")
        .unwrap()
        .unwrap();
    volume
        .set_object_comment(file, "wrong comment", time())
        .unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        baseline_contents(&mut volume)
    }));
    assert!(
        result.is_err(),
        "the content oracle must detect lost metadata"
    );
}
