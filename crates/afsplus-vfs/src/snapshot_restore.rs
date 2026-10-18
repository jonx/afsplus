// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Fresh-storage regular-file restoration from a pinned historical view.
//! The host checks backup and destination-scoped restore authority separately.
use super::*;
use afsplus_core::volume::{AttributeWriteMode, FileAllocationRange, FileEditLimits};

pub(super) const STAGE_ATTRIBUTE: &str = "system.restore.stage";

const COPY_BYTES: usize = 64 * 1024;
const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;
const MAX_ALLOCATION_RANGES: usize = 4096;

fn normalized_ranges(ranges: &[FileAllocationRange]) -> Vec<FileAllocationRange> {
    let mut out: Vec<FileAllocationRange> = Vec::new();
    for range in ranges {
        if let Some(last) = out.last_mut() {
            if last.unwritten == range.unwritten
                && last.offset.checked_add(last.length) == Some(range.offset)
            {
                last.length += range.length;
                continue;
            }
        }
        out.push(*range);
    }
    out
}

impl<D: BlockDevice> Vfs<D> {
    /// Restore into a new name only. A visible staging name is confined to the
    /// selected directory. Failures before publication remove it when storage
    /// remains writable; a failed cleanup names the retained stage explicitly.
    /// The final non-replacing rename publishes verified data and metadata in
    /// one transaction. A crash can leave a stage, never a partial final name.
    /// This limited consumer refuses links, directories and security metadata.
    pub fn snapshot_restore_file(
        &mut self,
        reader: Handle,
        source: ObjectId,
        parent: ObjectId,
        name: &str,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        self.restore_file_private(reader, source, parent, name, now, None)
    }

    pub(super) fn restore_file_private(
        &mut self,
        reader: Handle,
        source: ObjectId,
        parent: ObjectId,
        name: &str,
        now: Timespec,
        stage_root: Option<ObjectId>,
    ) -> Result<ObjectId, VfsError> {
        // Preflight must expose no target and must not truncate an old target.
        if self.mount_mode() != MountMode::ReadWrite {
            return Err(VfsError::ReadOnly);
        }
        self.sync_filesystem()?;
        let view = self
            .snapshot_readers
            .get(&reader)
            .ok_or(VfsError::Stale)?
            .clone();
        let captured = self
            .volume
            .snapshot_stat(&view, source)?
            .ok_or(VfsError::NotFound)?;
        if captured.object_type != ObjectType::File || captured.link_count != 1 {
            return Err(VfsError::NotSupported);
        }
        if self
            .volume
            .snapshot_security_descriptor(&view, source)?
            .ok_or(VfsError::NotFound)?
            .is_some()
        {
            return Err(VfsError::NotSupported);
        }
        if self.volume.lookup_in_directory(parent, name)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }
        let comment = self
            .volume
            .snapshot_object_comment(&view, source)?
            .ok_or(VfsError::NotFound)?;
        let keys = self
            .volume
            .snapshot_attribute_names(&view, source)?
            .ok_or(VfsError::NotFound)?;
        let keys: Vec<_> = keys.into_iter().filter(|k| k != STAGE_ATTRIBUTE).collect();
        let mut attributes = Vec::new();
        let mut metadata_bytes = comment.len();
        for key in keys {
            let value = self
                .volume
                .snapshot_attribute(&view, source, &key)?
                .ok_or(VfsError::NotFound)?
                .ok_or(VfsError::NotFound)?;
            metadata_bytes = metadata_bytes
                .checked_add(key.len())
                .and_then(|n| n.checked_add(value.len()))
                .ok_or(VfsError::Limit("restore metadata size overflow"))?;
            if metadata_bytes > MAX_METADATA_BYTES {
                return Err(VfsError::Limit("restore metadata exceeds 4 MiB"));
            }
            attributes.push((key, value));
        }
        let mut allocations = Vec::new();
        let mut cursor = 0;
        let block_size = self.volume.ident().geometry().block_size as u64;
        let rounded_size = captured
            .size_bytes
            .checked_add(block_size - 1)
            .ok_or(VfsError::Invalid)?
            / block_size
            * block_size;
        loop {
            let page = self
                .volume
                .snapshot_allocation_page(&view, source, cursor, 64)?;
            if allocations.len() + page.ranges.len() > MAX_ALLOCATION_RANGES {
                return Err(VfsError::Limit(
                    "restore allocation inventory exceeds 4096 ranges",
                ));
            }
            for range in &page.ranges {
                let end = range
                    .offset
                    .checked_add(range.length)
                    .ok_or(VfsError::Invalid)?;
                if !range.unwritten && end > rounded_size {
                    return Err(VfsError::NotSupported);
                }
            }
            allocations.extend(page.ranges);
            if page.eof {
                break;
            }
            if page.next <= cursor {
                return Err(VfsError::Corrupt(
                    "allocation cursor did not advance".into(),
                ));
            }
            cursor = page.next;
        }
        let target_key =
            afsplus_core::name_key::comparison_key(self.volume.ident(), name.as_bytes())?;
        let stage = (0..256)
            .map(|nonce| format!(".afsplus-restore-{reader:016x}-{source:016x}-{nonce:02x}"))
            .find_map(|stage| {
                match afsplus_core::name_key::comparison_key(self.volume.ident(), stage.as_bytes())
                {
                    Ok(key) if key == target_key => return None,
                    Ok(_) => {}
                    Err(error) => return Some(Err(error)),
                }
                match self.volume.lookup_in_directory(parent, &stage) {
                    Ok(None) => Some(Ok(stage)),
                    Ok(Some(_)) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .ok_or(VfsError::AlreadyExists)??;
        let target = match self.volume.create_file_in_directory(parent, &stage, &[], now) {
            Ok(object) => object,
            Err(error) => return Err(VfsError::Corrupt(format!("restore stage {stage}: creation failed: {error}; inspect retained stage before retry"))),
        };
        let result = (|| -> Result<(), VfsError> {
            self.volume.set_attributes(
                target,
                &[(
                    STAGE_ATTRIBUTE,
                    Some(stage_root.unwrap_or(target).to_le_bytes().as_slice()),
                )],
                AttributeWriteMode::Create,
                now,
            )?;
            let limits = FileEditLimits {
                max_blocks: 64,
                max_records: 64,
            };
            self.volume
                .truncate_file_bounded(target, captured.size_bytes, now, limits)?;
            // Respect the edit's64-block budget even on512-byte formats.
            let write_bytes = COPY_BYTES.min(block_size as usize * 64);
            let mut buffer = vec![0; COPY_BYTES];
            for range in &allocations {
                let mut offset = range.offset;
                let end = range
                    .offset
                    .checked_add(range.length)
                    .ok_or(VfsError::Invalid)?;
                while offset < end {
                    let length = (end - offset).min(write_bytes as u64);
                    if range.unwritten {
                        self.volume
                            .preallocate_file_bounded(target, offset, length, now, limits)?;
                        offset += length;
                    } else {
                        let bytes =
                            (captured.size_bytes.saturating_sub(offset)).min(length) as usize;
                        if bytes == 0 {
                            break;
                        }
                        let got = self.volume.snapshot_read_file_at(
                            &view,
                            source,
                            offset,
                            &mut buffer[..bytes],
                        )?;
                        if got != bytes {
                            return Err(VfsError::Corrupt("short historical restore read".into()));
                        }
                        self.volume.write_file_at_bounded(
                            target,
                            offset,
                            &buffer[..bytes],
                            now,
                            limits,
                        )?;
                        offset += bytes as u64;
                    }
                }
            }
            if !attributes.is_empty() {
                let changes: Vec<_> = attributes
                    .iter()
                    .map(|(key, value)| (key.as_str(), Some(value.as_slice())))
                    .collect();
                self.volume
                    .set_attributes(target, &changes, AttributeWriteMode::Create, now)?;
            }
            self.volume.set_object_comment(target, &comment, now)?;
            let preserved = PreservedMetadata::from(captured);
            self.volume.restore_object_metadata(target, preserved)?;
            let restored = self
                .volume
                .visible_metadata(target)?
                .ok_or(VfsError::NotFound)?;
            if PreservedMetadata::from(restored) != preserved
                || restored.size_bytes != captured.size_bytes
                || restored.allocated_bytes != captured.allocated_bytes
                || self.volume.object_comment(target)? != comment
                || self
                    .volume
                    .attribute_names(target)?
                    .into_iter()
                    .filter(|k| k != STAGE_ATTRIBUTE)
                    .collect::<Vec<_>>()
                    != attributes
                        .iter()
                        .map(|(key, _)| key.clone())
                        .collect::<Vec<_>>()
            {
                return Err(VfsError::Corrupt(
                    "restore metadata verification failed".into(),
                ));
            }
            let mut restored_allocations = Vec::new();
            let mut cursor = 0;
            loop {
                let page = self.volume.file_allocation_page(target, cursor, 64)?;
                if restored_allocations.len() + page.ranges.len() > MAX_ALLOCATION_RANGES {
                    return Err(VfsError::Limit(
                        "restored allocation inventory exceeds 4096 ranges",
                    ));
                }
                restored_allocations.extend(page.ranges);
                if page.eof {
                    break;
                }
                if page.next <= cursor {
                    return Err(VfsError::Corrupt(
                        "restored allocation cursor did not advance".into(),
                    ));
                }
                cursor = page.next;
            }
            if normalized_ranges(&restored_allocations) != normalized_ranges(&allocations) {
                return Err(VfsError::Corrupt(
                    "restore allocation verification failed".into(),
                ));
            }
            for (key, value) in &attributes {
                if self.volume.attribute(target, key)?.as_ref() != Some(value) {
                    return Err(VfsError::Corrupt(
                        "restore attribute verification failed".into(),
                    ));
                }
            }
            // Verify bytes independently through the restored object's live map.
            let mut check = vec![0; COPY_BYTES];
            let mut offset = 0;
            while offset < captured.size_bytes {
                let n = (captured.size_bytes - offset).min(COPY_BYTES as u64) as usize;
                if self
                    .volume
                    .snapshot_read_file_at(&view, source, offset, &mut buffer[..n])?
                    != n
                    || self.volume.read_file_at(target, offset, &mut check[..n])? != n
                    || buffer[..n] != check[..n]
                {
                    return Err(VfsError::Corrupt(
                        "restore content verification failed".into(),
                    ));
                }
                offset += n as u64;
            }
            self.volume.sync()?;
            self.volume
                .publish_restored_object(parent, &stage, name, now)?;
            self.volume.sync()?;
            Ok(())
        })();
        if let Err(error) = result {
            // Do not remove a completed target if a publication barrier failed.
            match self.volume.lookup_in_directory(parent, &stage) {
                Ok(Some(object)) if object == target => {
                    if let Err(cleanup) = self
                        .volume
                        .delete_file(parent, &stage, now)
                        .and_then(|_| self.volume.sync())
                    {
                        return Err(VfsError::Corrupt(format!(
                            "restore failed: {error}; retained stage {stage}: {cleanup}"
                        )));
                    }
                }
                Ok(None) => {}
                other => {
                    return Err(VfsError::Corrupt(format!(
                        "restore failed: {error}; inspect stage {stage}: {other:?}"
                    )))
                }
            }
            return Err(error);
        }
        Ok(target)
    }
}
