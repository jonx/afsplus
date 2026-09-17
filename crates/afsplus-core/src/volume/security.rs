//! Security preservation container: opaque, versioned descriptors attached to
//! objects, and the projection rule that keeps a simple host from weakening
//! them. The core stores and returns descriptor bytes; it never evaluates them.
use super::*;
use afsplus_format::ident::INCOMPAT_SECURITY_DESCRIPTORS;
use afsplus_format::object::{SecurityRef, SECURITY_REF_PROJECTION_DIVERGED};
use afsplus_format::security::{segment_count, SECURITY_CHAIN};
use chain::{load_chain, retire_chain, stage_chain, ChainRef, NewChain};

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

fn chain_ref(reference: SecurityRef) -> ChainRef {
    ChainRef {
        first_block: reference.first_block,
        total_len: reference.total_len,
        segment_count: reference.segment_count,
    }
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
    let (blocks, content) = load_chain(
        dev,
        geometry,
        &SECURITY_CHAIN,
        object_id,
        chain_ref(reference),
        max_generation,
    )?;
    Ok((
        blocks,
        SecurityDescriptor {
            format: content.format,
            version: content.version,
            projection_diverged: reference.flags & SECURITY_REF_PROJECTION_DIVERGED != 0,
            bytes: content.bytes,
        },
    ))
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
        let first_block = stage_chain(
            &mut self.dev,
            tx,
            &SECURITY_CHAIN,
            object_id,
            (descriptor.format, descriptor.version, &descriptor.bytes),
            reference.segment_count,
            generation,
            writes,
        )?;
        Ok(Some(SecurityRef {
            first_block,
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
        retire_chain(
            &mut self.dev,
            tx,
            &geometry,
            &SECURITY_CHAIN,
            record.object_id,
            chain_ref(reference),
            self.checkpoint.generation,
        )
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
        let old = record.security.map(chain_ref);
        self.replace_chain(
            record,
            &SECURITY_CHAIN,
            old,
            descriptor.map(|(format, version, bytes)| NewChain {
                format,
                version,
                bytes,
                segment_count: count,
            }),
            |record, staged| {
                let mut record = record.with_security(staged.map(|chain| SecurityRef {
                    first_block: chain.first_block,
                    total_len: chain.total_len,
                    segment_count: chain.segment_count,
                    flags: 0,
                }));
                record.changed = now;
                record
            },
        )
    }
}
