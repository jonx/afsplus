//! Extended attributes: named opaque values attached to an object. The whole
//! set of one object is one immutable owned chain (`chain.rs`); every change
//! writes a new chain and retires the old one in the commit that publishes
//! the new record. The core stores names and bytes; it gives no namespace a
//! meaning.
use super::*;
use afsplus_format::attrs::{
    decode_attribute_set, encode_attribute_set, segment_count, validate_attribute_name,
    ATTRIBUTE_CHAIN, ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION, ATTRIBUTE_VALUE_MAX_BYTES,
};
use afsplus_format::object::AttributeRef;
use chain::{load_chain, retire_chain, stage_chain, ChainRef, NewChain};
use std::collections::BTreeMap;

/// What a write does when the attribute exists already, or does not.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum AttributeWriteMode {
    /// Create or replace.
    #[default]
    Upsert,
    /// `AlreadyExists` when the attribute exists.
    Create,
    /// `NotFound` when the attribute does not exist.
    Replace,
}

type AttributeMap = BTreeMap<String, Vec<u8>>;

/// Entries of a stored set, in stored order.
pub(crate) type AttributeEntries = Vec<(String, Vec<u8>)>;

fn chain_ref(reference: AttributeRef) -> ChainRef {
    ChainRef {
        first_block: reference.first_block,
        total_len: reference.total_len,
        segment_count: reference.segment_count,
    }
}

/// The set `changes` makes of `before`: `Some(value)` writes under `mode`,
/// `None` removes and is `NotFound` when the attribute is absent. Changes
/// apply in order, so a later entry sees an earlier one.
fn apply_attribute_changes(
    before: &AttributeMap,
    changes: &[(&str, Option<&[u8]>)],
    mode: AttributeWriteMode,
) -> Result<AttributeMap, CoreError> {
    let mut after = before.clone();
    for (name, value) in changes {
        validate_attribute_name(name)
            .map_err(|_| CoreError::InvalidMetadata("attribute name out of range"))?;
        match value {
            None => {
                after.remove(*name).ok_or(CoreError::NotFound)?;
            }
            Some(value) => {
                if value.len() > ATTRIBUTE_VALUE_MAX_BYTES {
                    return Err(CoreError::InvalidMetadata("attribute value out of range"));
                }
                match (mode, after.contains_key(*name)) {
                    (AttributeWriteMode::Create, true) => return Err(CoreError::AlreadyExists),
                    (AttributeWriteMode::Replace, false) => return Err(CoreError::NotFound),
                    _ => {}
                }
                after.insert((*name).to_owned(), value.to_vec());
            }
        }
    }
    Ok(after)
}

/// The stored form of a set and the segments it takes; `None` for an empty
/// set, which an object stores as no chain at all.
fn encode_attributes(
    set: &AttributeMap,
    block_size: usize,
) -> Result<Option<(Vec<u8>, u16)>, CoreError> {
    if set.is_empty() {
        return Ok(None);
    }
    let entries: Vec<(&str, &[u8])> = set
        .iter()
        .map(|(name, value)| (name.as_str(), value.as_slice()))
        .collect();
    let bytes = encode_attribute_set(&entries)
        .map_err(|_| CoreError::InvalidMetadata("attribute set out of range"))?;
    let count = segment_count(bytes.len() as u32, block_size)
        .ok_or(CoreError::InvalidMetadata("attribute set out of range"))?;
    Ok(Some((bytes, count)))
}

/// Blocks and entries of one attribute chain, validated against its
/// reference and as a set. Anything short of a whole valid set is `Corrupt`.
pub(crate) fn load_attribute_chain<D: BlockDevice>(
    dev: &mut D,
    geometry: &afsplus_format::geometry::Geometry,
    object_id: u64,
    reference: AttributeRef,
    max_generation: u64,
) -> Result<(Vec<u64>, AttributeEntries), CoreError> {
    let (blocks, content) = load_chain(
        dev,
        geometry,
        &ATTRIBUTE_CHAIN,
        object_id,
        chain_ref(reference),
        max_generation,
    )?;
    if (content.format, content.version) != (ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION) {
        return Err(CoreError::Corrupt(format!(
            "object {object_id} attribute set has unknown format {} version {}",
            content.format, content.version
        )));
    }
    let entries = decode_attribute_set(&content.bytes)
        .map_err(|error| CoreError::Corrupt(format!("object {object_id} attribute set: {error}")))?
        .into_iter()
        .map(|(name, value)| (name.to_owned(), value.to_vec()))
        .collect();
    Ok((blocks, entries))
}

impl<D: BlockDevice> Volume<D> {
    fn load_attributes(&mut self, record: &ObjectRecord) -> Result<AttributeMap, CoreError> {
        // A record the open window staged may name a chain that window
        // wrote, stamped with its generation; nothing else carries it.
        let generation = self
            .window
            .as_ref()
            .map_or(self.checkpoint.generation, |window| window.generation);
        self.load_attributes_up_to(record, generation)
    }

    fn load_attributes_up_to(
        &mut self,
        record: &ObjectRecord,
        generation: u64,
    ) -> Result<AttributeMap, CoreError> {
        let Some(reference) = record.attributes else {
            return Ok(AttributeMap::new());
        };
        let geometry = self.ident.geometry();
        let (_, entries) = load_attribute_chain(
            &mut self.dev,
            &geometry,
            record.object_id,
            reference,
            generation,
        )?;
        Ok(entries.into_iter().collect())
    }

    fn attribute_target(&mut self, object_id: u64) -> Result<ObjectRecord, CoreError> {
        self.ensure_public_object_id(object_id)?;
        let record = self.visible_record(object_id)?.ok_or(CoreError::NotFound)?;
        if record.object_type == ObjectType::Internal {
            return Err(CoreError::NotFound);
        }
        Ok(record)
    }

    /// The value of attribute `name` of `object_id`, or `None` when the
    /// object has no such attribute.
    pub fn attribute(&mut self, object_id: u64, name: &str) -> Result<Option<Vec<u8>>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::Attribute, |volume| {
            let record = volume.attribute_target(object_id)?;
            Ok(volume.load_attributes(&record)?.remove(name))
        })
    }

    /// The attribute names of `object_id`, ascending by name bytes.
    pub fn attribute_names(&mut self, object_id: u64) -> Result<Vec<String>, CoreError> {
        self.trace_api(crate::flight::ApiMethod::AttributeNames, |volume| {
            let record = volume.attribute_target(object_id)?;
            Ok(volume.load_attributes(&record)?.into_keys().collect())
        })
    }

    /// Apply `changes` to the attributes of `object_id` in one commit: a
    /// power cut leaves every change or none. `Some(value)` writes under
    /// `mode`; `None` removes and is `NotFound` when the attribute is absent.
    /// Changes apply in order, so a later entry sees an earlier one. A batch
    /// that leaves the set as it was commits nothing. The change time
    /// advances; the modification time stays, the content did not change.
    pub fn set_attributes(
        &mut self,
        object_id: u64,
        changes: &[(&str, Option<&[u8]>)],
        mode: AttributeWriteMode,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::SetAttributes, |volume| {
            volume.ensure_window_closed()?;
            metadata::validate_time(now)?;
            let record = volume.attribute_target(object_id)?;
            let before = volume.load_attributes(&record)?;
            let after = apply_attribute_changes(&before, changes, mode)?;
            if after == before {
                return Ok(());
            }
            let encoded = encode_attributes(&after, volume.dev.block_size())?;
            let old = record.attributes.map(chain_ref);
            volume.replace_chain(
                record,
                &ATTRIBUTE_CHAIN,
                old,
                encoded.as_ref().map(|(bytes, count)| NewChain {
                    format: ATTRIBUTE_SET_FORMAT,
                    version: ATTRIBUTE_SET_VERSION,
                    bytes,
                    segment_count: *count,
                }),
                |record, staged| {
                    let mut record = record.with_attributes(staged.map(|chain| AttributeRef {
                        first_block: chain.first_block,
                        total_len: chain.total_len,
                        segment_count: chain.segment_count,
                    }));
                    record.changed = now;
                    record
                },
            )
        })
    }

    /// [`Self::set_attributes`] staged in the open window (ADR-121): the
    /// new set is written to fresh blocks now and published by the window's
    /// commit, together with everything else the window holds, so a power
    /// cut leaves every staged change of the window or none. The window
    /// becomes one an fsync must commit rather than log, as for a directory.
    /// A symlink and the root are [`CoreError::PrototypeLimit`]: the caller
    /// commits the window and uses the immediate path.
    pub fn window_set_attributes(
        &mut self,
        object_id: u64,
        changes: &[(&str, Option<&[u8]>)],
        mode: AttributeWriteMode,
        now: Timespec,
    ) -> Result<(), CoreError> {
        self.trace_api(crate::flight::ApiMethod::WindowSetAttributes, |volume| {
            metadata::validate_time(now)?;
            volume.ensure_public_object_id(object_id)?;
            let mut window = volume.take_or_open_window()?;
            match volume.window_set_attributes_in(&mut window, object_id, changes, mode, now) {
                Ok(()) => {
                    volume.window = Some(window);
                    Ok(())
                }
                // Nothing was allocated or written: the window stands.
                Err((false, error)) => {
                    volume.window = Some(window);
                    Err(error)
                }
                Err((true, error)) => {
                    volume.window_poisoned = true;
                    volume.flight_window_event(
                        window.generation,
                        crate::flight::EventKind::WindowFailed,
                        None,
                    );
                    volume.flight_window_event(
                        window.generation,
                        crate::flight::EventKind::WindowClosed,
                        None,
                    );
                    Err(error)
                }
            }
        })
    }

    /// The error says whether the window was changed before it happened.
    fn window_set_attributes_in(
        &mut self,
        window: &mut OpenWindow,
        object_id: u64,
        changes: &[(&str, Option<&[u8]>)],
        mode: AttributeWriteMode,
        now: Timespec,
    ) -> Result<(), (bool, CoreError)> {
        let untouched = |error: CoreError| (false, error);
        let record = self
            .window_editable_record(window, object_id)
            .map_err(untouched)?;
        let generation = window.generation;
        let before = self
            .load_attributes_up_to(&record, generation)
            .map_err(untouched)?;
        let after = apply_attribute_changes(&before, changes, mode).map_err(untouched)?;
        if after == before {
            return Ok(());
        }
        let encoded = encode_attributes(&after, self.dev.block_size()).map_err(untouched)?;
        let changed = |error: CoreError| (true, error);
        let mut writes = Vec::new();
        let staged = match &encoded {
            Some((bytes, count)) => {
                let first_block = stage_chain(
                    &mut self.dev,
                    &mut window.tx,
                    &ATTRIBUTE_CHAIN,
                    object_id,
                    (ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION, bytes),
                    *count,
                    generation,
                    &mut writes,
                )
                .map_err(changed)?;
                Some(AttributeRef {
                    first_block,
                    total_len: bytes.len() as u32,
                    segment_count: *count,
                })
            }
            None => None,
        };
        // Written now, like the window's file data, and covered by the
        // barrier of the commit that first names them.
        for (lba, block) in &writes {
            self.dev
                .write_block(*lba, block)
                .map_err(|e| changed(e.into()))?;
        }
        // A chain this window staged before is released; the committed one
        // it replaced was retired then. Otherwise the committed chain is
        // retired now, as the immediate path does.
        if window
            .pending
            .staged_attribute_chains
            .contains_key(&object_id)
        {
            self.release_staged_attributes(&mut window.tx, &mut window.pending, object_id)
                .map_err(changed)?;
        } else if let Some(previous) = record.attributes {
            let geometry = self.ident.geometry();
            retire_chain(
                &mut self.dev,
                &mut window.tx,
                &geometry,
                &ATTRIBUTE_CHAIN,
                object_id,
                chain_ref(previous),
                self.checkpoint.generation,
            )
            .map_err(changed)?;
        }
        if staged.is_some() {
            window
                .pending
                .staged_attribute_chains
                .insert(object_id, writes.iter().map(|(lba, _)| *lba).collect());
        }
        let mut record = record.with_attributes(staged);
        record.changed = now;
        self.window_stage_record(window, record).map_err(changed)
    }

    /// Releases the attribute chain the batch staged for `object_id`, if it
    /// staged one. Nothing durable names those blocks.
    pub(super) fn release_staged_attributes(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        object_id: u64,
    ) -> Result<(), CoreError> {
        if let Some(blocks) = pending.staged_attribute_chains.remove(&object_id) {
            for lba in blocks {
                tx.release_uncommitted(&mut self.dev, lba)?;
            }
        }
        Ok(())
    }

    /// [`Self::retire_attributes`] inside a batch: a chain the batch staged
    /// is released, and the committed chain it replaced was retired when it
    /// was staged.
    pub(super) fn retire_attributes_in_batch(
        &mut self,
        tx: &mut TxAllocator,
        pending: &mut PendingBatch,
        record: &ObjectRecord,
    ) -> Result<(), CoreError> {
        if pending
            .staged_attribute_chains
            .contains_key(&record.object_id)
        {
            return self.release_staged_attributes(tx, pending, record.object_id);
        }
        self.retire_attributes(tx, record)
    }

    /// Stage the initial managed marker alongside a new object record.
    pub(super) fn create_restore_attributes(
        &mut self,
        tx: &mut TxAllocator,
        object_id: u64,
        root: u64,
        generation: u64,
        writes: &mut Vec<(u64, Vec<u8>)>,
    ) -> Result<AttributeRef, CoreError> {
        let marker = root.to_le_bytes();
        let bytes = encode_attribute_set(&[("system.restore.stage", marker.as_slice())])
            .map_err(|_| CoreError::InvalidMetadata("restore marker attribute out of range"))?;
        let count = segment_count(bytes.len() as u32, self.dev.block_size()).ok_or(
            CoreError::InvalidMetadata("restore marker attribute out of range"),
        )?;
        let first_block = stage_chain(
            &mut self.dev,
            tx,
            &ATTRIBUTE_CHAIN,
            object_id,
            (ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION, &bytes),
            count,
            generation,
            writes,
        )?;
        Ok(AttributeRef {
            first_block,
            total_len: bytes.len() as u32,
            segment_count: count,
        })
    }

    /// Remove the internal restore marker inside the namespace publication transaction.
    pub(super) fn publish_restore_attributes(
        &mut self,
        tx: &mut TxAllocator,
        record: &ObjectRecord,
        generation: u64,
        writes: &mut Vec<(u64, Vec<u8>)>,
    ) -> Result<Option<AttributeRef>, CoreError> {
        let mut attributes = self.load_attributes(record)?;
        if attributes.get("system.restore.stage").map(Vec::as_slice)
            != Some(record.object_id.to_le_bytes().as_slice())
        {
            return Ok(record.attributes);
        }
        attributes.remove("system.restore.stage");
        self.retire_attributes(tx, record)?;
        if attributes.is_empty() {
            return Ok(None);
        }
        let entries: Vec<_> = attributes
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_slice()))
            .collect();
        let bytes = encode_attribute_set(&entries)
            .map_err(|_| CoreError::InvalidMetadata("attribute set out of range"))?;
        let count = segment_count(bytes.len() as u32, self.dev.block_size())
            .ok_or(CoreError::InvalidMetadata("attribute set out of range"))?;
        let first_block = stage_chain(
            &mut self.dev,
            tx,
            &ATTRIBUTE_CHAIN,
            record.object_id,
            (ATTRIBUTE_SET_FORMAT, ATTRIBUTE_SET_VERSION, &bytes),
            count,
            generation,
            writes,
        )?;
        Ok(Some(AttributeRef {
            first_block,
            total_len: bytes.len() as u32,
            segment_count: count,
        }))
    }

    /// Copy the attribute chain of `source` into a fresh chain owned by
    /// `object_id`, inside the caller's transaction; the twin of
    /// `copy_security_descriptor`.
    pub(super) fn copy_attributes(
        &mut self,
        tx: &mut TxAllocator,
        source: &ObjectRecord,
        object_id: u64,
        generation: u64,
        writes: &mut Vec<(u64, Vec<u8>)>,
    ) -> Result<Option<AttributeRef>, CoreError> {
        let Some(reference) = source.attributes else {
            return Ok(None);
        };
        let geometry = self.ident.geometry();
        let (_, content) = load_chain(
            &mut self.dev,
            &geometry,
            &ATTRIBUTE_CHAIN,
            source.object_id,
            chain_ref(reference),
            self.checkpoint.generation,
        )?;
        let first_block = stage_chain(
            &mut self.dev,
            tx,
            &ATTRIBUTE_CHAIN,
            object_id,
            (content.format, content.version, &content.bytes),
            reference.segment_count,
            generation,
            writes,
        )?;
        Ok(Some(AttributeRef {
            first_block,
            ..reference
        }))
    }

    /// Retire the attribute chain of an object that leaves the namespace for
    /// good. A damaged chain never blocks the deletion; see
    /// `retire_security_descriptor`.
    pub(super) fn retire_attributes(
        &mut self,
        tx: &mut TxAllocator,
        record: &ObjectRecord,
    ) -> Result<(), CoreError> {
        let Some(reference) = record.attributes else {
            return Ok(());
        };
        let geometry = self.ident.geometry();
        retire_chain(
            &mut self.dev,
            tx,
            &geometry,
            &ATTRIBUTE_CHAIN,
            record.object_id,
            chain_ref(reference),
            self.checkpoint.generation,
        )
    }
}
