//! The C side states the format twice: `spec/afsplus_format.h` and the
//! private macros of the portable reader. Neither may drift from the Rust
//! codecs. Two probes print every constant; each printed name must be paired
//! here with the Rust constant, or with a length measured from a block the
//! Rust codec just encoded, and the values must be equal.
#![cfg(unix)]
use afsplus_format::checkpoint::Checkpoint;
use afsplus_format::header::{block_type, HEADER_SIZE, HEADER_VERSION};
use afsplus_format::ident::{self, FeatureFlags, Identification, NameKeyAlgorithm};
use afsplus_format::object::{self, ObjectRecord, ObjectType, SecurityRef};
use afsplus_format::security::{segment_capacity, MAX_SECURITY_DESCRIPTOR_BYTES};
use afsplus_format::tree::{TreeItem, TreeKind, TreeNode, MAX_TREE_KEY_BYTES, MAX_TREE_LEVEL};
use afsplus_format::{bitmap, extent, geometry, intent_log, Timespec};
use std::collections::BTreeMap;
use std::{fs, path::PathBuf, process::Command};

const BS: usize = afsplus_format::DEFAULT_BLOCK_SIZE;

struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn payload_len(block: &[u8]) -> u64 {
    u64::from(u32::from_le_bytes(block[24..28].try_into().unwrap()))
}

fn file_record() -> ObjectRecord {
    ObjectRecord {
        object_id: 16,
        object_type: ObjectType::File,
        flags: 0,
        link_count: 1,
        size_bytes: 0,
        allocated_bytes: 0,
        created: Timespec::default(),
        modified: Timespec::default(),
        changed: Timespec::default(),
        protection: 0,
        content_generation: 1,
        data_root: 0,
        data_blocks: 0,
        security: None,
    }
}

fn probe(scratch: &Scratch, source: &str) -> BTreeMap<String, u64> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let executable = scratch.0.join(source.replace(".c", ""));
    let output = Command::new(std::env::var_os("CC").unwrap_or_else(|| "cc".into()))
        .args(["-std=c99", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(repo.join("api"))
        .arg("-I")
        .arg(repo.join("spec"))
        .arg(repo.join("portable/c/tests").join(source))
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let printed = Command::new(&executable).output().unwrap();
    assert!(printed.status.success());
    String::from_utf8(printed.stdout)
        .unwrap()
        .lines()
        .map(|line| {
            let (name, value) = line.split_once(' ').unwrap();
            (name.to_owned(), value.parse().unwrap())
        })
        .collect()
}

fn object_type_wire(kind: ObjectType) -> u64 {
    // The wire value of a type is private to the codec; read it back from an
    // encoded record. Symlinks and internal objects have no fixed encoder.
    let mut record = file_record();
    record.object_type = kind;
    if kind == ObjectType::Directory {
        record.data_root = 99;
    }
    u64::from(record.encode(BS, 1).unwrap()[HEADER_SIZE + 8])
}

fn tree_kind_wire(kind: TreeKind) -> u64 {
    u64::from(TreeNode::leaf(kind, 0).encode(BS, 1).unwrap()[HEADER_SIZE])
}

#[test]
fn every_c_format_constant_equals_the_rust_codec() {
    let scratch =
        Scratch(std::env::temp_dir().join(format!("afsplus-c-constants-{}", std::process::id())));
    fs::create_dir(&scratch.0).unwrap();

    let ident = Identification {
        uuid: [7; 16],
        block_shift: afsplus_format::DEFAULT_BLOCK_SHIFT,
        checksum_algorithm: afsplus_format::crc32c::CHECKSUM_CRC32C,
        region_size: 256,
        log_slots: 0,
        features: FeatureFlags::default(),
        name_key_algorithm: NameKeyAlgorithm::UnicodeNfc,
        unicode_version: ident::UNICODE_VERSION_16_0_0,
        total_blocks: 1024,
        checkpoint_slots: [1, 2],
        metadata_start: 9,
        label: "Pin".into(),
    };
    let checkpoint = Checkpoint {
        uuid: [7; 16],
        generation: 5,
        root_object_id: afsplus_format::OBJECT_ROOT,
        object_map_block: 10,
        allocation_root_block: 12,
        reclaim_root_block: 11,
        next_object_id: 20,
        committed_tx_id: 5,
        free_blocks_total: 800,
        flags: 0,
        shared_extent_root_block: 0,
        label: String::new(),
        snapshot_roots: None,
    };
    let mut with_roots = checkpoint.clone();
    with_roots.snapshot_roots = Some(afsplus_format::checkpoint::SnapshotRoots {
        registry: 30,
        lifetimes: 31,
    });
    let mut labelled = checkpoint.clone();
    labelled.label = "L".into();
    let label_offset = {
        let plain = checkpoint.encode(BS).unwrap();
        let named = labelled.encode(BS).unwrap();
        // The length byte is the first payload byte that differs.
        (HEADER_SIZE..BS)
            .find(|at| plain[*at] != named[*at] && *at >= HEADER_SIZE + 88)
            .unwrap() as u64
            - HEADER_SIZE as u64
    };
    let secured = file_record().with_security(Some(SecurityRef {
        first_block: 77,
        total_len: 1,
        segment_count: 1,
        flags: 0,
    }));
    let empty_leaf = TreeNode::leaf(TreeKind::ObjectMap, 0);
    let mut one_item = empty_leaf.clone();
    one_item.items.push(TreeItem {
        key: vec![1],
        value: vec![2],
    });
    one_item.subtree_items = 1;
    let tree_fixed = payload_len(&empty_leaf.encode(BS, 1).unwrap());
    let tree_item_fixed = payload_len(&one_item.encode(BS, 1).unwrap()) - tree_fixed - 2;
    let object_payload = payload_len(&file_record().encode(BS, 1).unwrap());
    let empty_bitmap = bitmap::BitmapPage::all_free(0, 0, 0, 8);
    let bitmap_fixed = payload_len(&empty_bitmap.encode(BS, 1).unwrap()) - 1;

    let header: Vec<(&str, u64)> = vec![
        ("AFSP_FORMAT_EPOCH", u64::from(afsplus_format::FORMAT_EPOCH)),
        (
            "AFSP_DEFAULT_BLOCK_SHIFT",
            u64::from(afsplus_format::DEFAULT_BLOCK_SHIFT),
        ),
        ("AFSP_DEFAULT_BLOCK_SIZE", BS as u64),
        (
            "AFSP_NAME_MAX_UTF8_BYTES",
            afsplus_format::NAME_MAX_UTF8_BYTES as u64,
        ),
        ("AFSP_LABEL_MAX_UTF8_BYTES", ident::LABEL_MAX_BYTES as u64),
        // Measured: a checkpoint whose label is one byte long has that byte
        // as the first nonzero byte after the 96 fixed bytes plus eight.
        ("AFSP_CHECKPOINT_LABEL_OFFSET", label_offset),
        (
            "AFSP_CHECKPOINT_PAYLOAD_BYTES",
            payload_len(&checkpoint.encode(BS).unwrap()),
        ),
        (
            "AFSP_CHECKPOINT_SNAPSHOT_PAYLOAD_BYTES",
            payload_len(&with_roots.encode(BS).unwrap()),
        ),
        ("AFSP_OBJECT_INVALID", afsplus_format::OBJECT_INVALID),
        ("AFSP_OBJECT_ROOT", afsplus_format::OBJECT_ROOT),
        (
            "AFSP_OBJECT_ORPHAN_DIRECTORY",
            afsplus_format::OBJECT_ORPHAN_DIRECTORY,
        ),
        ("AFSP_MAGIC_U64", afsplus_format::FS_MAGIC),
        ("AFSP_INCOMPAT_INTENT_LOG", ident::INCOMPAT_INTENT_LOG),
        (
            "AFSP_INCOMPAT_INTENT_LOG_DATA_UPDATES",
            ident::INCOMPAT_INTENT_LOG_DATA_UPDATES,
        ),
        (
            "AFSP_INCOMPAT_PERSISTENT_SNAPSHOTS",
            ident::INCOMPAT_PERSISTENT_SNAPSHOTS,
        ),
        (
            "AFSP_INCOMPAT_SECURITY_DESCRIPTORS",
            ident::INCOMPAT_SECURITY_DESCRIPTORS,
        ),
        (
            "AFSP_RO_COMPAT_SHARED_EXTENTS",
            ident::RO_COMPAT_SHARED_EXTENTS,
        ),
        (
            "AFSP_RO_COMPAT_ORPHAN_DIRECTORY",
            ident::RO_COMPAT_ORPHAN_DIRECTORY,
        ),
        ("AFSP_COMPAT_DATA_POLICY", ident::COMPAT_DATA_POLICY),
        ("AFSP_OBJECT_FILE", object_type_wire(ObjectType::File)),
        (
            "AFSP_OBJECT_DIRECTORY",
            object_type_wire(ObjectType::Directory),
        ),
        // Literal: the symlink and internal types have no fixed-record
        // encoder to measure; the symlink codec test pins 3.
        ("AFSP_OBJECT_SYMLINK", 3),
        ("AFSP_OBJECT_INTERNAL", 4),
        (
            "AFSP_EXTENT_FLAG_UNWRITTEN",
            u64::from(extent::EXTENT_UNWRITTEN),
        ),
        ("AFSP_EXTENT_FLAG_SHARED", u64::from(extent::EXTENT_SHARED)),
        (
            "AFSP_OBJECT_FLAG_EXTENT_TREE",
            u64::from(object::OBJECT_FLAG_EXTENT_TREE),
        ),
        (
            "AFSP_OBJECT_FLAG_DATA_IN_PLACE",
            u64::from(object::OBJECT_FLAG_DATA_IN_PLACE),
        ),
        (
            "AFSP_OBJECT_FLAG_SECURITY_REF",
            u64::from(object::OBJECT_FLAG_SECURITY_REF),
        ),
        ("sizeof_afsp_timespec_wire", Timespec::WIRE_SIZE as u64),
        (
            "sizeof_afsp_extent_value_wire",
            extent::EXTENT_VALUE_LEN as u64,
        ),
    ];
    let reader: Vec<(&str, u64)> = vec![
        ("AFSPR_HEADER_SIZE", HEADER_SIZE as u64),
        ("AFSPR_CHECKSUM_OFFSET", 28),
        ("AFSPR_HEADER_VERSION", u64::from(HEADER_VERSION)),
        (
            "AFSPR_BLOCK_TYPE_IDENT",
            u64::from(block_type::IDENTIFICATION),
        ),
        (
            "AFSPR_BLOCK_TYPE_CHECKPOINT",
            u64::from(block_type::CHECKPOINT),
        ),
        ("AFSPR_BLOCK_TYPE_OBJECT", u64::from(block_type::OBJECT)),
        (
            "AFSPR_BLOCK_TYPE_SECURITY",
            u64::from(block_type::SECURITY_DESCRIPTOR),
        ),
        ("AFSPR_BLOCK_TYPE_TREE", u64::from(block_type::TREE_NODE)),
        ("AFSPR_BLOCK_TYPE_INTENT", u64::from(block_type::INTENT_LOG)),
        ("AFSPR_BLOCK_TYPE_BITMAP", u64::from(block_type::BITMAP)),
        (
            "AFSPR_BLOCK_TYPE_REGION_DESCRIPTOR",
            u64::from(block_type::REGION_DESCRIPTOR),
        ),
        (
            "AFSPR_CHECKSUM_CRC32C",
            u64::from(afsplus_format::crc32c::CHECKSUM_CRC32C),
        ),
        (
            "AFSPR_IDENT_CURRENT_PAYLOAD",
            payload_len(&ident.encode(BS).unwrap()),
        ),
        (
            "AFSPR_CHECKPOINT_PAYLOAD",
            payload_len(&checkpoint.encode(BS).unwrap()),
        ),
        (
            "AFSPR_OBJECT_FIRST_DYNAMIC",
            afsplus_format::OBJECT_FIRST_DYNAMIC,
        ),
        (
            "AFSPR_MIN_REGION_BLOCKS",
            u64::from(geometry::MIN_REGION_BLOCKS),
        ),
        (
            "AFSPR_MAX_REGION_BLOCKS",
            u64::from(geometry::MAX_REGION_BLOCKS),
        ),
        (
            "AFSPR_BITMAP_PAGE_BLOCKS",
            u64::from(bitmap::BITMAP_PAGE_BLOCKS),
        ),
        ("AFSPR_BOOTSTRAP_BLOCKS", geometry::BOOTSTRAP_BLOCKS),
        (
            "AFSPR_DESCRIPTOR_SLOTS",
            u64::from(geometry::DESCRIPTOR_SLOTS),
        ),
        ("AFSPR_BITMAP_SLOTS", u64::from(geometry::BITMAP_SLOTS)),
        ("AFSPR_TREE_FIXED_PAYLOAD", tree_fixed),
        ("AFSPR_TREE_ITEM_FIXED", tree_item_fixed),
        ("AFSPR_TREE_MAX_LEVEL", u64::from(MAX_TREE_LEVEL)),
        ("AFSPR_TREE_MAX_KEY", MAX_TREE_KEY_BYTES as u64),
        (
            "AFSPR_TREE_KIND_OBJECT_MAP",
            tree_kind_wire(TreeKind::ObjectMap),
        ),
        (
            "AFSPR_TREE_KIND_DIRECTORY",
            tree_kind_wire(TreeKind::Directory),
        ),
        (
            "AFSPR_TREE_KIND_EXTENT_MAP",
            tree_kind_wire(TreeKind::ExtentMap),
        ),
        (
            "AFSPR_TREE_KIND_ALLOCATION_ROOT",
            tree_kind_wire(TreeKind::AllocationRoot),
        ),
        (
            "AFSPR_ALLOCATION_VALUE_SIZE",
            geometry::ALLOCATION_ROOT_VALUE_BYTES as u64,
        ),
        // Literal: the region descriptor has no public fixed-length constant.
        ("AFSPR_REGION_DESCRIPTOR_FIXED", 16),
        ("AFSPR_BITMAP_FIXED", bitmap_fixed),
        ("AFSPR_OBJECT_PAYLOAD", object_payload),
        (
            "AFSPR_SECURITY_REF_SIZE",
            payload_len(&secured.encode(BS, 1).unwrap()) - object_payload,
        ),
        (
            "AFSPR_SECURITY_SEGMENT_FIXED",
            (BS - HEADER_SIZE - segment_capacity(BS)) as u64,
        ),
        ("AFSPR_MAX_DIRECT_BLOCKS", object::MAX_EXTENT_BLOCKS),
        ("AFSPR_EXTENT_VALUE_SIZE", extent::EXTENT_VALUE_LEN as u64),
        // Literals: the intent-log record lengths are private to its codec.
        ("AFSPR_LOG_FIXED_PAYLOAD", 32),
        ("AFSPR_LOG_OP_FIXED", 64),
        ("AFSPR_LOG_CURRENT_VERSION", 3),
        ("AFSPR_LOG_MAX_EXTENTS", intent_log::MAX_LOG_EXTENTS as u64),
        ("AFSPR_LOG_MAX_OPS", intent_log::MAX_LOG_OPS as u64),
        (
            "AFSPR_MAX_SECURITY_DESCRIPTOR_BYTES",
            u64::from(MAX_SECURITY_DESCRIPTOR_BYTES),
        ),
    ];

    for (source, expected) in [
        ("spec_probe.c", header),
        ("reader_constants_probe.c", reader),
    ] {
        let printed = probe(&scratch, source);
        let expected: BTreeMap<String, u64> = expected
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect();
        // Equality of the whole maps: a constant the probe prints and this
        // test does not pair, or the reverse, is a failure too.
        assert_eq!(printed, expected, "{source}");
    }
}
