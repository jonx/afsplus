//! Security preservation container: opaque, versioned descriptors attached to
//! objects, and the projection rule that keeps a simple host from weakening
//! them. The core stores and returns descriptor bytes; it never evaluates them.
use super::*;
use afsplus_format::ident::INCOMPAT_SECURITY_DESCRIPTORS;
use afsplus_format::object::{SecurityRef, SECURITY_REF_PROJECTION_DIVERGED};
use afsplus_format::security::{segment_capacity, segment_count, SecuritySegment};

/// What a protection edit does to an object that carries a descriptor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SecurityProjectionPolicy {
    /// Refuse the edit: the host cannot show that the new protection value
    /// agrees with a descriptor it does not evaluate.
    #[default]
    Strict,
    /// Apply the edit, keep every descriptor byte and record durably that
    /// the projection diverged, so a host that evaluates the descriptor
    /// reconciles the two.
    Preserve,
}

/// A descriptor as stored: format identity, format version and opaque bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityDescriptor {
    pub format: u32,
    pub version: u16,
    /// The protection field was edited under
    /// [`SecurityProjectionPolicy::Preserve`] after this descriptor was set.
    pub projection_diverged: bool,
    pub bytes: Vec<u8>,
}

/// One walk of a descriptor chain.
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
    pub descriptor: Option<SecurityDescriptor>,
    /// Why the walk stopped, when it stopped early.
    pub damage: Option<String>,
}

/// Walk the chain a reference names, proving one segment at a time. Device
/// errors propagate; a damaged chain is a value, not an error, so the paths
/// that must free an object are not blocked by the bytes it points at.
pub(crate) fn walk_descriptor_chain<D: BlockDevice>(
    dev: &mut D,
    geometry: &afsplus_format::geometry::Geometry,
    object_id: u64,
    reference: SecurityRef,
    max_generation: u64,
) -> Result<ChainWalk, CoreError> {
    let mut block = vec![0u8; dev.block_size()];
    let mut blocks = Vec::with_capacity(reference.segment_count as usize);
    let mut bytes = Vec::with_capacity(reference.total_len as usize);
    let mut identity = None;
    // One commit writes every segment of a chain, so they all carry the
    // generation of the first. A stale segment of an earlier descriptor of
    // the same object and size matches every other field and not this one.
    let mut chain_generation = None;
    let mut lba = reference.first_block;
    for index in 0..reference.segment_count {
        if !geometry.is_allocatable(lba) || blocks.contains(&lba) {
            return Ok(ChainWalk {
                blocks,
                descriptor: None,
                damage: Some(format!(
                    "object {object_id} security segment {index} at invalid block {lba}"
                )),
            });
        }
        dev.read_block(lba, &mut block)?;
        let mismatch = match SecuritySegment::decode(&block) {
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
                descriptor: None,
                damage: Some(format!(
                    "object {object_id} security segment {index} does not match its reference"
                )),
            });
        }
    }
    let Some((format, version)) = identity else {
        return Ok(ChainWalk {
            blocks,
            descriptor: None,
            damage: Some("security reference without segments".into()),
        });
    };
    Ok(ChainWalk {
        blocks,
        descriptor: Some(SecurityDescriptor {
            format,
            version,
            projection_diverged: reference.flags & SECURITY_REF_PROJECTION_DIVERGED != 0,
            bytes,
        }),
        damage: None,
    })
}

/// Blocks of one descriptor chain, validated against its reference. A chain
/// that does not validate in full is `Corrupt`: descriptor bytes are returned
/// whole or not at all.
pub(crate) fn load_descriptor_chain<D: BlockDevice>(
    dev: &mut D,
    geometry: &afsplus_format::geometry::Geometry,
    object_id: u64,
    reference: SecurityRef,
    max_generation: u64,
) -> Result<(Vec<u64>, SecurityDescriptor), CoreError> {
    let walk = walk_descriptor_chain(dev, geometry, object_id, reference, max_generation)?;
    match walk.descriptor {
        Some(descriptor) => Ok((walk.blocks, descriptor)),
        None => Err(CoreError::Corrupt(walk.damage.unwrap_or_else(|| {
            format!("object {object_id} security chain is damaged")
        }))),
    }
}

impl<D: BlockDevice> Volume<D> {
    pub(super) fn security_descriptors_enabled(&self) -> bool {
        self.ident.features.incompat & INCOMPAT_SECURITY_DESCRIPTORS != 0
    }

    /// Select what protection edits do to descriptor-bearing objects. Runtime
    /// host policy: it resets to `Strict` at every mount.
    pub fn set_security_projection_policy(&mut self, policy: SecurityProjectionPolicy) {
        self.trace_api_infallible(
            crate::flight::ApiMethod::SetSecurityProjectionPolicy,
            |volume| volume.security_projection = policy,
        )
    }

    /// The descriptor of `object_id`, or `None` when it carries none.
    pub fn security_descriptor(
        &mut self,
        object_id: u64,
    ) -> Result<Option<SecurityDescriptor>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::SecurityDescriptor, |volume| {
            volume.ensure_public_object_id(object_id)?;
            let record = volume.read_object(object_id)?.ok_or(CoreError::NotFound)?;
            let Some(reference) = record.security else {
                return Ok(None);
            };
            let geometry = volume.ident.geometry();
            let generation = volume.checkpoint.generation;
            let (_, descriptor) = load_descriptor_chain(
                &mut volume.dev,
                &geometry,
                object_id,
                reference,
                generation,
            )?;
            Ok(Some(descriptor))
        })
    }

    /// Attach or replace the descriptor of `object_id`. The bytes are opaque;
    /// `format` is a nonzero registry identity. Replacing clears the
    /// projection-diverged mark, because the caller supplies the descriptor
    /// that matches the object's present state.
    pub fn set_security_descriptor(
        &mut self,
        object_id: u64,
        format: u32,
        version: u16,
        bytes: &[u8],
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetSecurityDescriptor, |volume| {
            volume.replace_security_descriptor(object_id, Some((format, version, bytes)), now)
        })
    }

    /// Remove the descriptor of `object_id`. This is the explicit downgrade:
    /// no other operation discards descriptor bytes of a live object.
    pub fn clear_security_descriptor(
        &mut self,
        object_id: u64,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(
            crate::flight::ApiMethod::ClearSecurityDescriptor,
            |volume| volume.replace_security_descriptor(object_id, None, now),
        )
    }

    /// The projection rule, applied by every path that changes `protection`.
    pub(super) fn project_protection(
        &self,
        record: &mut ObjectRecord,
        protection: u32,
    ) -> Result<(), CoreError> {
        if let Some(reference) = record.security.as_mut() {
            if record.protection != protection {
                match self.security_projection {
                    SecurityProjectionPolicy::Strict => {
                        return Err(CoreError::SecurityProjectionRefused)
                    }
                    SecurityProjectionPolicy::Preserve => {
                        reference.flags |= SECURITY_REF_PROJECTION_DIVERGED;
                    }
                }
            }
        }
        record.protection = protection;
        Ok(())
    }

    /// Copy the descriptor chain of `source` into a fresh chain owned by
    /// `object_id`, inside the caller's transaction. A chain has exactly one
    /// owner, so a copy never shares the source's segments; format identity,
    /// version, bytes and the projection-divergence mark are carried over
    /// unchanged. The segment writes join `writes`, so the new chain and the
    /// record that references it are published by one commit.
    pub(super) fn copy_security_descriptor(
        &mut self,
        tx: &mut TxAllocator,
        source: &ObjectRecord,
        object_id: u64,
        generation: u64,
        writes: &mut Vec<(u64, Vec<u8>)>,
    ) -> Result<Option<SecurityRef>, CoreError> {
        let Some(reference) = source.security else {
            return Ok(None);
        };
        let geometry = self.ident.geometry();
        let (_, descriptor) = load_descriptor_chain(
            &mut self.dev,
            &geometry,
            source.object_id,
            reference,
            self.checkpoint.generation,
        )?;
        let block_size = self.dev.block_size();
        let count = reference.segment_count;
        let lbas = (0..count)
            .map(|_| tx.allocate(&mut self.dev))
            .collect::<Result<Vec<_>, _>>()?;
        let capacity = segment_capacity(block_size);
        for (index, chunk) in descriptor.bytes.chunks(capacity).enumerate() {
            let segment = SecuritySegment {
                object_id,
                format: descriptor.format,
                version: descriptor.version,
                total_len: reference.total_len,
                index: index as u16,
                count,
                next: lbas.get(index + 1).copied().unwrap_or(0),
                bytes: chunk,
            };
            writes.push((lbas[index], segment.encode(block_size, generation)?));
        }
        Ok(Some(SecurityRef {
            first_block: lbas[0],
            ..reference
        }))
    }

    /// Retire the descriptor chain of an object that leaves the namespace
    /// for good, or whose descriptor is replaced. Callers retire the object
    /// record themselves.
    ///
    /// A damaged chain never blocks the operation: an object must stay
    /// deletable whatever the bytes it points at look like, or a single
    /// corrupt segment would pin its name, its records and its data
    /// forever. That includes a first segment outside the volume: the record
    /// is admitted and the chain is damaged from its first link. Only the
    /// segments consistent with the reference are freed (see [`ChainWalk`]
    /// for what that judgement covers and what it leaves to a crafted
    /// image); the remainder stays allocated, where the checker reports it
    /// as a block owned by nothing. Leaking beats freeing a block that may
    /// still belong to something else: on uncertainty, ADR-021 quarantines
    /// or leaks rather than reusing early.
    pub(super) fn retire_security_descriptor(
        &mut self,
        tx: &mut TxAllocator,
        record: &ObjectRecord,
    ) -> Result<(), CoreError> {
        let Some(reference) = record.security else {
            return Ok(());
        };
        let geometry = self.ident.geometry();
        let walk = walk_descriptor_chain(
            &mut self.dev,
            &geometry,
            record.object_id,
            reference,
            self.checkpoint.generation,
        )?;
        for lba in walk.blocks {
            tx.retire(&mut self.dev, lba)?;
        }
        Ok(())
    }

    fn replace_security_descriptor(
        &mut self,
        object_id: u64,
        descriptor: Option<(u32, u16, &[u8])>,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.ensure_window_closed()?;
        self.ensure_public_object_id(object_id)?;
        metadata::validate_time(now)?;
        if !self.security_descriptors_enabled() {
            return Err(CoreError::FeatureDisabled(
                "security-descriptors feature is not enabled on this volume",
            ));
        }
        let block_size = self.dev.block_size();
        let count = match descriptor {
            Some((format, _, bytes)) => {
                let length = u32::try_from(bytes.len()).ok();
                let count = length.and_then(|length| segment_count(length, block_size));
                match (format, count) {
                    (0, _) | (_, None) => {
                        return Err(CoreError::InvalidMetadata(
                            "security descriptor format or length out of range",
                        ))
                    }
                    (_, Some(count)) => count,
                }
            }
            None => 0,
        };
        let record = self.read_object(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type == ObjectType::Internal {
            return Err(CoreError::NotFound);
        }
        if descriptor.is_none() && record.security.is_none() {
            return Ok(());
        }
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
        let mut reference = None;
        if let Some((format, version, bytes)) = descriptor {
            let lbas = (0..count)
                .map(|_| tx.allocate(&mut self.dev))
                .collect::<Result<Vec<_>, _>>()?;
            let capacity = segment_capacity(block_size);
            for (index, chunk) in bytes.chunks(capacity).enumerate() {
                let segment = SecuritySegment {
                    object_id,
                    format,
                    version,
                    total_len: bytes.len() as u32,
                    index: index as u16,
                    count,
                    next: lbas.get(index + 1).copied().unwrap_or(0),
                    bytes: chunk,
                };
                writes.push((lbas[index], segment.encode(block_size, generation)?));
            }
            reference = Some(SecurityRef {
                first_block: lbas[0],
                total_len: bytes.len() as u32,
                segment_count: count,
                flags: 0,
            });
        }
        self.retire_security_descriptor(&mut tx, &record)?;
        let mut new_record = record.with_security(reference);
        new_record.changed = now;
        let new_lba = tx.allocate(&mut self.dev)?;
        tx.retire(&mut self.dev, record_lba)?;
        // A symlink at its longest target has no room for the reference;
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
