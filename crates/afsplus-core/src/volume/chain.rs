//! Owned chains: the immutable block chains one object owns, walked, staged
//! and retired the same way whatever their kind. The security descriptor
//! container is the first kind.
use super::*;
use afsplus_format::chain::{segment_capacity, ChainKind, ChainSegment};

/// Where a chain starts and what it must add up to, as the owning record
/// states it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChainRef {
    pub first_block: u64,
    pub total_len: u32,
    pub segment_count: u16,
}

/// Content of a chain proven in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChainContent {
    pub format: u32,
    pub version: u16,
    pub bytes: Vec<u8>,
}

/// A chain about to be written: segment format, version and the content,
/// with the segment count the content occupies.
#[derive(Debug, Clone, Copy)]
pub(crate) struct NewChain<'a> {
    pub format: u32,
    pub version: u16,
    pub bytes: &'a [u8],
    pub segment_count: u16,
}

/// One walk of an owned chain.
pub(crate) struct ChainWalk {
    /// Segments whose content is consistent with this object's reference:
    /// valid magic and checksum, this owner, the expected position, matching
    /// identity, a committed generation and the generation of the first
    /// segment, from the first segment up to the first inconsistent link. A
    /// block past that point is never accepted, whoever else may own it. The
    /// walk judges bytes, not allocator ownership: a data block that holds a
    /// valid segment image, reached through a forged and resealed next
    /// pointer, passes. That needs a crafted image, and is the trust level
    /// the extent trees have.
    pub blocks: Vec<u64>,
    /// Present only when every segment of the chain was proven.
    pub content: Option<ChainContent>,
    /// Why the walk stopped, when it stopped early.
    pub damage: Option<String>,
}

/// Walk the chain a reference names, proving one segment at a time. Device
/// errors propagate; a damaged chain is a value, not an error, so the paths
/// that must free an object are not blocked by the bytes it points at.
pub(crate) fn walk_chain<D: BlockDevice>(
    dev: &mut D,
    geometry: &afsplus_format::geometry::Geometry,
    kind: &ChainKind,
    object_id: u64,
    reference: ChainRef,
    max_generation: u64,
) -> Result<ChainWalk, CoreError> {
    let label = kind.label;
    let mut block = vec![0u8; dev.block_size()];
    let mut blocks = Vec::with_capacity(reference.segment_count as usize);
    let mut bytes = Vec::with_capacity(reference.total_len as usize);
    let mut identity = None;
    // One commit writes every segment of a chain, so they all carry the
    // generation of the first. A stale segment of an earlier chain of the
    // same object and size matches every other field and not this one.
    let mut chain_generation = None;
    let mut lba = reference.first_block;
    for index in 0..reference.segment_count {
        if !geometry.is_allocatable(lba) || blocks.contains(&lba) {
            return Ok(ChainWalk {
                blocks,
                content: None,
                damage: Some(format!(
                    "object {object_id} {label} segment {index} at invalid block {lba}"
                )),
            });
        }
        dev.read_block(lba, &mut block)?;
        let mismatch = match ChainSegment::decode(kind, &block) {
            Err(_) => true,
            Ok((segment, generation)) => {
                let first = *identity.get_or_insert((segment.format, segment.version));
                let ok = segment.object_id == object_id
                    && segment.index == index
                    && segment.count == reference.segment_count
                    && segment.total_len == reference.total_len
                    && (segment.format, segment.version) == first
                    && generation != 0
                    && generation <= max_generation
                    && *chain_generation.get_or_insert(generation) == generation;
                if ok {
                    blocks.push(lba);
                    bytes.extend_from_slice(segment.bytes);
                    lba = segment.next;
                }
                !ok
            }
        };
        if mismatch {
            return Ok(ChainWalk {
                blocks,
                content: None,
                damage: Some(format!(
                    "object {object_id} {label} segment {index} does not match its reference"
                )),
            });
        }
    }
    let Some((format, version)) = identity else {
        return Ok(ChainWalk {
            blocks,
            content: None,
            damage: Some(format!("{label} reference without segments")),
        });
    };
    Ok(ChainWalk {
        blocks,
        content: Some(ChainContent {
            format,
            version,
            bytes,
        }),
        damage: None,
    })
}

/// Blocks and content of one chain, validated against its reference. A chain
/// that does not validate in full is `Corrupt`: content is returned whole or
/// not at all.
pub(crate) fn load_chain<D: BlockDevice>(
    dev: &mut D,
    geometry: &afsplus_format::geometry::Geometry,
    kind: &ChainKind,
    object_id: u64,
    reference: ChainRef,
    max_generation: u64,
) -> Result<(Vec<u64>, ChainContent), CoreError> {
    let walk = walk_chain(dev, geometry, kind, object_id, reference, max_generation)?;
    match walk.content {
        Some(content) => Ok((walk.blocks, content)),
        None => Err(CoreError::Corrupt(walk.damage.unwrap_or_else(|| {
            format!("object {object_id} {} chain is damaged", kind.label)
        }))),
    }
}

/// Allocate and encode a fresh chain of `segment_count` segments owned by
/// `object_id`. The segment writes join `writes`, so the chain and the record
/// that references it are published by one commit. Returns the first block.
#[allow(clippy::too_many_arguments)]
pub(crate) fn stage_chain<D: BlockDevice>(
    dev: &mut D,
    tx: &mut TxAllocator,
    kind: &ChainKind,
    object_id: u64,
    content: (u32, u16, &[u8]),
    segment_count: u16,
    generation: u64,
    writes: &mut Vec<(u64, Vec<u8>)>,
) -> Result<u64, CoreError> {
    let (format, version, bytes) = content;
    let block_size = dev.block_size();
    let lbas = (0..segment_count)
        .map(|_| tx.allocate(dev))
        .collect::<Result<Vec<_>, _>>()?;
    let capacity = segment_capacity(block_size);
    for (index, chunk) in bytes.chunks(capacity).enumerate() {
        let segment = ChainSegment {
            object_id,
            format,
            version,
            total_len: bytes.len() as u32,
            index: index as u16,
            count: segment_count,
            next: lbas.get(index + 1).copied().unwrap_or(0),
            bytes: chunk,
        };
        writes.push((lbas[index], segment.encode(kind, block_size, generation)?));
    }
    Ok(lbas[0])
}

/// Retire the segments of a chain that are consistent with its reference.
/// A damaged chain never blocks the caller; see
/// `Volume::retire_security_descriptor` for why the remainder is leaked.
pub(crate) fn retire_chain<D: BlockDevice>(
    dev: &mut D,
    tx: &mut TxAllocator,
    geometry: &afsplus_format::geometry::Geometry,
    kind: &ChainKind,
    object_id: u64,
    reference: ChainRef,
    max_generation: u64,
) -> Result<(), CoreError> {
    let walk = walk_chain(dev, geometry, kind, object_id, reference, max_generation)?;
    for lba in walk.blocks {
        tx.retire(dev, lba)?;
    }
    Ok(())
}

impl<D: BlockDevice> Volume<D> {
    /// Replace, attach or remove one owned chain of `record`'s object in one
    /// commit: the new chain is staged, the old one retired, and the record
    /// `apply` builds from the staged reference is published with them. A
    /// power cut leaves the old chain with the old record or the new chain
    /// with the new one.
    pub(super) fn replace_chain(
        &mut self,
        record: ObjectRecord,
        kind: &ChainKind,
        old: Option<ChainRef>,
        new: Option<NewChain<'_>>,
        apply: impl FnOnce(ObjectRecord, Option<ChainRef>) -> ObjectRecord,
    ) -> Result<(), CoreError> {
        let object_id = record.object_id;
        let record_lba = self.object_record_lba(object_id)?.ok_or_else(|| {
            CoreError::Corrupt(format!("object {object_id} missing from object map"))
        })?;
        let generation = self.next_generation()?;
        let mut tx = TxAllocator::begin(
            &mut self.dev,
            &self.ident.geometry(),
            &self.checkpoint,
            self.other_checkpoint.as_ref(),
            generation,
            self.reclaim_batch_blocks,
            self.alloc_rover_region,
        )?
        .with_tree_cache_pages(self.tree_cache_pages);
        self.protect_emergency_headroom(&mut tx);

        let mut writes = Vec::new();
        let mut staged = None;
        if let Some(new) = new {
            let first_block = stage_chain(
                &mut self.dev,
                &mut tx,
                kind,
                object_id,
                (new.format, new.version, new.bytes),
                new.segment_count,
                generation,
                &mut writes,
            )?;
            staged = Some(ChainRef {
                first_block,
                total_len: new.bytes.len() as u32,
                segment_count: new.segment_count,
            });
        }
        if let Some(old) = old {
            let geometry = self.ident.geometry();
            retire_chain(
                &mut self.dev,
                &mut tx,
                &geometry,
                kind,
                object_id,
                old,
                self.checkpoint.generation,
            )?;
        }
        let new_record = apply(record, staged);
        let new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;
        // A symlink at its longest target has no room for another field;
        // the encoder refuses it before anything is published.
        writes.push((
            new_lba,
            self.encode_preserving_target(new_record, generation)?,
        ));
        let key = object_map::key(object_id);
        let value = object_map::value(new_lba)?;
        let mutation = mutate_many(
            &mut self.dev,
            &self.ident.geometry(),
            &mut tx,
            self.checkpoint.object_map_block,
            object_map::spec(self.checkpoint.generation),
            generation,
            &[TreeOperation::Upsert {
                key: &key,
                value: &value,
            }],
        )?;
        writes.extend(mutation.writes);
        self.commit_transaction(
            generation,
            self.checkpoint.next_object_id,
            tx,
            Vec::new(),
            writes,
            mutation.root_lba,
        )
    }
}
