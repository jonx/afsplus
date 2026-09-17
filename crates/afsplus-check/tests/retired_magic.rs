//! A block carrying a retired magic is not a block of this format
//! ([ADR-115](../../../adr/ADR-115-retire-unwritten-surface.md)). Its
//! procedure table claims that `afsplus_check::explain` gives such a block no
//! identity and that nothing claims it; this test holds that claim, which
//! nothing else did.
//!
//! The second half of the claim, that an allocated block nothing claims is
//! reported as owned by nothing, is the general leak rule of
//! [`zero_tail.rs`](zero_tail.rs) and the explain tests: `is_unowned` is
//! decided by the allocation bit and the roles, never by the block's content,
//! so a retired magic cannot change it.
use afsplus_block::{BlockDevice, MemoryBackend};
use afsplus_check::check_device;
use afsplus_check::explain::{Allocation, Explainer};
use afsplus_core::{mkfs, mount, MkfsParams, NamePolicy};
use afsplus_format::header::BlockHeader;
use afsplus_format::{Timespec, OBJECT_ROOT};

const BLOCK: usize = 4096;

fn time(n: i64) -> Timespec {
    Timespec {
        seconds: n,
        nanoseconds: 0,
    }
}

#[test]
fn a_retired_magic_is_not_a_kind_and_claims_nothing() {
    let mut dev = MemoryBackend::new(BLOCK, 512);
    mkfs(
        &mut dev,
        &MkfsParams {
            uuid: [0x33; 16],
            label: "RetiredMagic".into(),
            region_size: 512,
            reclaim_caps: Default::default(),
            log_slots: 0,
            shared_extents: true,
            data_policy: true,
            name_policy: NamePolicy::Sensitive,
            timestamp: time(1),
        },
    )
    .unwrap();
    let mut volume = mount(dev).unwrap();
    volume
        .create_file_in_directory(OBJECT_ROOT, "file", b"content", time(2))
        .unwrap();
    let mut dev = volume.into_device();

    let clean = check_device(&mut dev);
    assert!(clean.errors.is_empty(), "{:?}", clean.errors);
    let explainer = Explainer::load(&mut dev).unwrap();
    let free = (0..dev.total_blocks())
        .find(|lba| explainer.allocation(*lba) == Allocation::Free)
        .expect("a free block");

    // A well-formed block of each retired kind: right magic, right header
    // version, valid checksum. Only the magic makes it unreadable.
    for magic in [*b"AFSD", *b"AFSM", *b"AFSR"] {
        let mut block = vec![0u8; BLOCK];
        block[0..4].copy_from_slice(&magic);
        BlockHeader {
            block_type: u32::from_le_bytes(magic),
            flags: 0,
            owner: OBJECT_ROOT,
            generation: 7,
            payload_len: 8,
        }
        .seal(&mut block);
        assert!(
            BlockHeader::verify(&block, u32::from_le_bytes(magic)).is_ok(),
            "the probe block is not well formed"
        );
        dev.write_block(free, &block).unwrap();

        let explainer = Explainer::load(&mut dev).unwrap();
        assert_eq!(explainer.problems, Vec::<String>::new());
        let explanation = explainer.explain_block(&mut dev, free).unwrap();
        let named = String::from_utf8_lossy(&magic).into_owned();
        // No identity: the magic is not one this format knows.
        assert_eq!(explanation.identity, None, "{named} was given an identity");
        // And nothing claims it.
        assert_eq!(explanation.roles, Vec::new(), "{named} was claimed");
        assert_eq!(explanation.allocation, Allocation::Free, "{named}");
        assert!(
            !explanation.is_unowned(),
            "{named}: a free block is not a leak"
        );
        // The volume's verdict is unchanged: what a free block holds is not
        // a claim about the filesystem.
        let report = check_device(&mut dev);
        assert_eq!(report.errors, clean.errors, "{named}");
        assert_eq!(report.warnings, clean.warnings, "{named}");
    }
}
