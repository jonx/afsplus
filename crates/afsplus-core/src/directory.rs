//! Typed directory adapter for the shared COW tree.

use afsplus_block::BlockDevice;
use afsplus_format::dir::DirEntry;
use afsplus_format::geometry::Geometry;
use afsplus_format::ident::Identification;
use afsplus_format::tree::{TreeKind, TreeNode};
use afsplus_format::{le, validate_name, OBJECT_INVALID};

use crate::name_key::{validate_entry_key, COMPARISON_KEY_MAX_BYTES};
use crate::tree::{
    lookup, read_range, visit_tree_nodes, visit_tree_nodes_bounded, TreeSpec, TreeSummary,
};
use crate::CoreError;

const VALUE_FIXED: usize = 16;

pub struct LoadedDirectory {
    pub owner: u64,
    pub entries: Vec<DirEntry>,
    pub tree_blocks: Vec<u64>,
    pub summary: TreeSummary,
}

pub fn spec(owner: u64, max_generation: u64) -> TreeSpec {
    TreeSpec {
        kind: TreeKind::Directory,
        owner,
        max_generation,
    }
}

pub fn empty_leaf(owner: u64) -> TreeNode {
    TreeNode::leaf(TreeKind::Directory, owner)
}

pub fn encode_entry(
    ident: &Identification,
    entry: &DirEntry,
) -> Result<(Vec<u8>, Vec<u8>), CoreError> {
    validate_logical_entry(ident, entry)?;
    if entry.name.len() > u16::MAX as usize {
        return Err(CoreError::Corrupt(
            "directory name exceeds wire length".into(),
        ));
    }
    let mut value = vec![0u8; VALUE_FIXED + entry.name.len()];
    le::put_u16(&mut value[0..2], entry.name.len() as u16);
    value[2] = entry.child_type_hint;
    le::put_u64(&mut value[8..16], entry.child_id);
    value[VALUE_FIXED..].copy_from_slice(&entry.name);
    Ok((entry.key.clone(), value))
}

pub fn lookup_entry<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
    ident: &Identification,
    key: &[u8],
) -> Result<Option<DirEntry>, CoreError> {
    let (value, _) = lookup(dev, geo, root_lba, spec(owner, max_generation), key)?;
    value
        .map(|value| decode_entry(ident, key, &value))
        .transpose()
}

/// Validates only the root node for bounded mount. Descendant paths are
/// checked on lookup; the checker uses [`load_all`] for exhaustive coverage.
pub fn validate_root<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
) -> Result<(), CoreError> {
    crate::tree::check_tree_lba(geo, root_lba)?;
    let mut block = vec![0u8; geo.block_size];
    dev.read_block(root_lba, &mut block)?;
    let (node, generation) = TreeNode::decode(&block)
        .map_err(|error| CoreError::Corrupt(format!("directory root {root_lba}: {error}")))?;
    crate::tree::validate_node_identity(
        &node,
        generation,
        spec(owner, max_generation),
        None,
        root_lba,
    )?;
    crate::tree::validate_node_range(&node, None, None, true)
}

pub fn load_all<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
    ident: &Identification,
) -> Result<LoadedDirectory, CoreError> {
    let mut entries = Vec::new();
    let mut tree_blocks = Vec::new();
    let summary = visit_tree_nodes(
        dev,
        geo,
        root_lba,
        spec(owner, max_generation),
        |lba, node| {
            tree_blocks.push(lba);
            if node.is_leaf() {
                for item in &node.items {
                    entries.push(decode_entry(ident, &item.key, &item.value)?);
                }
            }
            Ok(())
        },
    )?;
    if entries.len() as u64 != summary.items {
        return Err(CoreError::Corrupt(
            "directory leaf count does not match tree summary".into(),
        ));
    }
    Ok(LoadedDirectory {
        owner,
        entries,
        tree_blocks,
        summary,
    })
}

/// Reads a bounded ordinal page without materializing the whole directory.
pub fn read_page<D: BlockDevice>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    tree: TreeSpec,
    ident: &Identification,
    start: u64,
    limit: usize,
) -> Result<(Vec<DirEntry>, u64), CoreError> {
    let page = read_range(dev, geo, root_lba, tree, start, limit)?;
    let entries = page
        .items
        .into_iter()
        .map(|(key, value)| decode_entry(ident, &key, &value))
        .collect::<Result<Vec<_>, _>>()?;
    Ok((entries, page.total_items))
}

/// Exhaustively validates the directory tree and yields each typed entry in
/// binary key order without retaining the directory contents. The verifier's
/// memory is bounded by the format's maximum tree height; only one decoded
/// entry is materialized for the callback at a time.
pub fn visit_entries<D, F>(
    dev: &mut D,
    geo: &Geometry,
    root_lba: u64,
    owner: u64,
    max_generation: u64,
    ident: &Identification,
    mut visitor: F,
) -> Result<TreeSummary, CoreError>
where
    D: BlockDevice,
    F: FnMut(&DirEntry) -> Result<(), CoreError>,
{
    let mut entries = 0u64;
    let summary = visit_tree_nodes_bounded(
        dev,
        geo,
        root_lba,
        spec(owner, max_generation),
        |_, node| {
            if node.is_leaf() {
                for item in &node.items {
                    let entry = decode_entry(ident, &item.key, &item.value)?;
                    visitor(&entry)?;
                    entries = entries.checked_add(1).ok_or_else(|| {
                        CoreError::Corrupt("directory entry count overflow".into())
                    })?;
                }
            }
            Ok(())
        },
    )?;
    if entries != summary.items {
        return Err(CoreError::Corrupt(
            "directory leaf count does not match tree summary".into(),
        ));
    }
    Ok(summary)
}

fn decode_entry(ident: &Identification, key: &[u8], encoded: &[u8]) -> Result<DirEntry, CoreError> {
    if encoded.len() < VALUE_FIXED {
        return Err(CoreError::Corrupt(
            "directory leaf value is truncated".into(),
        ));
    }
    let name_len = le::get_u16(&encoded[0..2]) as usize;
    if encoded.len() != VALUE_FIXED + name_len {
        return Err(CoreError::Corrupt(
            "directory leaf name length mismatch".into(),
        ));
    }
    if encoded[3..8] != [0; 5] {
        return Err(CoreError::Corrupt(
            "directory leaf reserved bytes are nonzero".into(),
        ));
    }
    let entry = DirEntry {
        key: key.to_vec(),
        name: encoded[VALUE_FIXED..].to_vec(),
        child_type_hint: encoded[2],
        child_id: le::get_u64(&encoded[8..16]),
    };
    validate_logical_entry(ident, &entry)?;
    Ok(entry)
}

fn validate_logical_entry(ident: &Identification, entry: &DirEntry) -> Result<(), CoreError> {
    validate_name(&entry.name).map_err(CoreError::Format)?;
    if entry.key.is_empty() || entry.key.len() > COMPARISON_KEY_MAX_BYTES {
        return Err(CoreError::Corrupt(
            "directory comparison key length is out of range".into(),
        ));
    }
    validate_entry_key(ident, &entry.key, &entry.name)?;
    if entry.child_id == OBJECT_INVALID {
        return Err(CoreError::Corrupt(
            "directory entry references invalid object".into(),
        ));
    }
    if !matches!(entry.child_type_hint, 1 | 2) {
        return Err(CoreError::Corrupt(
            "directory entry has invalid child type hint".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use afsplus_block::{BlockDevice, MemoryBackend};
    use afsplus_format::dir::DirEntry;
    use afsplus_format::geometry::Geometry;
    use afsplus_format::ident::{
        FeatureFlags, Identification, NameKeyAlgorithm, UNICODE_VERSION_16_0_0,
    };
    use afsplus_format::{crc32c::CHECKSUM_CRC32C, DEFAULT_BLOCK_SHIFT};

    use super::{empty_leaf, encode_entry, load_all, lookup_entry, spec, visit_entries};
    use crate::allocation_root::ReservedTreePool;
    use crate::cow_tree::{mutate_many, mutate_many_with_cache_limit, TreeOperation};

    fn test_ident(geo: &Geometry, name_key_algorithm: NameKeyAlgorithm) -> Identification {
        Identification {
            uuid: [1; 16],
            block_shift: DEFAULT_BLOCK_SHIFT,
            checksum_algorithm: CHECKSUM_CRC32C,
            region_size: geo.region_size,
            log_slots: 0,
            features: FeatureFlags::default(),
            name_key_algorithm,
            unicode_version: if name_key_algorithm == NameKeyAlgorithm::LegacyIdentity {
                [0, 0, 0]
            } else {
                UNICODE_VERSION_16_0_0
            },
            total_blocks: geo.total_blocks,
            checkpoint_slots: [1, 2],
            metadata_start: geo.region0_reserved_blocks(),
            label: "directory-test".into(),
        }
    }

    #[test]
    fn typed_directory_crosses_leaf_and_internal_boundaries() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 4096,
            region_size: 4096,
        };
        let mut dev = MemoryBackend::new(4096, 4096);
        let ident = test_ident(&geo, NameKeyAlgorithm::LegacyIdentity);
        dev.write_block(100, &empty_leaf(1).encode(4096, 1).unwrap())
            .unwrap();
        let entries: Vec<_> = (0..1000u64)
            .map(|id| DirEntry {
                key: format!("file-{id:04}").into_bytes(),
                name: format!("file-{id:04}").into_bytes(),
                child_type_hint: 1,
                child_id: id + 100,
            })
            .collect();
        let encoded: Vec<_> = entries
            .iter()
            .map(|entry| encode_entry(&ident, entry))
            .collect::<Result<_, _>>()
            .unwrap();
        let operations: Vec<_> = encoded
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let mut pool = ReservedTreePool::new(100..300, &[100], &[]).unwrap();
        let mutation =
            mutate_many(&mut dev, &geo, &mut pool, 100, spec(1, 1), 2, &operations).unwrap();
        for (lba, block) in &mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }
        let loaded = load_all(&mut dev, &geo, mutation.root_lba, 1, 2, &ident).unwrap();
        assert_eq!(loaded.entries, entries);
        assert!(loaded.summary.nodes > 1);
        assert_eq!(
            lookup_entry(
                &mut dev,
                &geo,
                mutation.root_lba,
                1,
                2,
                &ident,
                b"file-0999",
            )
            .unwrap()
            .unwrap()
            .child_id,
            1099
        );
    }

    #[test]
    #[ignore = "explicit 100k-entry typed-directory scale qualification"]
    fn typed_directory_qualifies_one_hundred_thousand_entries_bounded() {
        let geo = Geometry {
            block_size: 4096,
            total_blocks: 16_384,
            region_size: 16_384,
        };
        let owner = 17;
        let ident = test_ident(&geo, NameKeyAlgorithm::UnicodeNfcCasefold);
        let mut dev = MemoryBackend::new(4096, geo.total_blocks);
        dev.write_block(100, &empty_leaf(owner).encode(4096, 1).unwrap())
            .unwrap();
        let entries: Vec<_> = (0..100_000u64)
            .map(|ordinal| {
                let id = (ordinal * 7_919) % 100_000;
                let name = format!("entry-{id:06}").into_bytes();
                DirEntry {
                    key: name.clone(),
                    name,
                    child_type_hint: if id.is_multiple_of(11) { 2 } else { 1 },
                    child_id: id + 100,
                }
            })
            .collect();
        let encoded: Vec<_> = entries
            .iter()
            .map(|entry| encode_entry(&ident, entry))
            .collect::<Result<_, _>>()
            .unwrap();
        let operations: Vec<_> = encoded
            .iter()
            .map(|(key, value)| TreeOperation::Upsert { key, value })
            .collect();
        let mut pool = ReservedTreePool::new(100..16_000, &[100], &[]).unwrap();
        let mutation_started = std::time::Instant::now();
        let mutation = mutate_many_with_cache_limit(
            &mut dev,
            &geo,
            &mut pool,
            100,
            spec(owner, 1),
            2,
            &operations,
            8,
        )
        .unwrap();
        let mutation_elapsed = mutation_started.elapsed();
        assert!(mutation.stats.staged_spill_writes > 0);
        assert!(mutation.stats.staged_spill_reloads > 0);
        assert!(mutation.stats.max_resident_staged_nodes <= 8);
        assert!(mutation.stats.max_live_decoded_nodes <= 2);
        for (lba, block) in &mutation.writes {
            dev.write_block(*lba, block).unwrap();
        }

        let mut visited = 0u64;
        let mut previous_name = None;
        let visit_started = std::time::Instant::now();
        let summary = visit_entries(
            &mut dev,
            &geo,
            mutation.root_lba,
            owner,
            2,
            &ident,
            |entry| {
                if let Some(previous) = &previous_name {
                    assert!(previous < &entry.name);
                }
                let id: u64 = std::str::from_utf8(&entry.name[6..])
                    .unwrap()
                    .parse()
                    .unwrap();
                assert_eq!(entry.child_id, id + 100);
                assert_eq!(
                    entry.child_type_hint,
                    if id.is_multiple_of(11) { 2 } else { 1 }
                );
                previous_name = Some(entry.name.clone());
                visited += 1;
                Ok(())
            },
        )
        .unwrap();
        let visit_elapsed = visit_started.elapsed();
        assert_eq!(visited, 100_000);
        assert_eq!(summary.items, 100_000);
        assert!(summary.height >= 3);
        for id in [0u64, 49_999, 99_999] {
            let name = format!("entry-{id:06}");
            let entry = lookup_entry(
                &mut dev,
                &geo,
                mutation.root_lba,
                owner,
                2,
                &ident,
                name.as_bytes(),
            )
            .unwrap()
            .unwrap();
            assert_eq!(entry.child_id, id + 100);
        }
        eprintln!(
            "100k typed directory: build {:?}, stream {:?}, height {}, nodes {}, reads {}, spill writes/reloads {}/{}, max staged/decoded {}/{}",
            mutation_elapsed,
            visit_elapsed,
            summary.height,
            summary.nodes,
            mutation.stats.device_reads,
            mutation.stats.staged_spill_writes,
            mutation.stats.staged_spill_reloads,
            mutation.stats.max_resident_staged_nodes,
            mutation.stats.max_live_decoded_nodes,
        );
    }
}
