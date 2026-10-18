// SPDX-License-Identifier: BSD-2-Clause
// Copyright (c) 2026 John Knipper
//! Caller-budgeted fresh restoration of a complete captured directory (ADR-128).
use super::*;
use afsplus_core::volume::{AttributeWriteMode, SnapshotHandle};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TreeRestoreLimits {
    pub max_nodes: usize,
    pub max_depth: usize,
    pub max_metadata_bytes: usize,
}
struct Node {
    source: ObjectId,
    parent: Option<usize>,
    name: String,
    metadata: ObjectMetadata,
    comment: String,
    attributes: Vec<(String, Vec<u8>)>,
}

impl<D: BlockDevice> Vfs<D> {
    fn restore_tree_plan(
        &mut self,
        view: &SnapshotHandle,
        source: ObjectId,
        limits: TreeRestoreLimits,
    ) -> Result<Vec<Node>, VfsError> {
        let mut pending = vec![(source, None, String::new(), 1usize)];
        let mut nodes: Vec<Node> = Vec::new();
        let mut seen = BTreeSet::new();
        let mut metadata_bytes = 0usize;
        while let Some((source, parent, name, depth)) = pending.pop() {
            if depth > limits.max_depth || nodes.len() >= limits.max_nodes {
                return Err(VfsError::Limit("restore tree node/depth budget exhausted"));
            }
            if !seen.insert(source) {
                return Err(VfsError::NotSupported);
            }
            let metadata = self
                .volume
                .snapshot_stat(view, source)?
                .ok_or(VfsError::NotFound)?;
            if !matches!(
                metadata.object_type,
                ObjectType::File | ObjectType::Directory
            ) || (metadata.object_type == ObjectType::File && metadata.link_count != 1)
                || self
                    .volume
                    .snapshot_security_descriptor(view, source)?
                    .ok_or(VfsError::NotFound)?
                    .is_some()
            {
                return Err(VfsError::NotSupported);
            }
            if nodes.is_empty() && metadata.object_type != ObjectType::Directory {
                return Err(VfsError::NotDirectory);
            }
            let comment = self
                .volume
                .snapshot_object_comment(view, source)?
                .ok_or(VfsError::NotFound)?;
            metadata_bytes = metadata_bytes
                .checked_add(name.len())
                .and_then(|n| n.checked_add(comment.len()))
                .ok_or(VfsError::Limit("restore tree metadata overflow"))?;
            if metadata_bytes > limits.max_metadata_bytes {
                return Err(VfsError::Limit("restore tree metadata budget exhausted"));
            }
            let mut attributes = Vec::new();
            for key in self
                .volume
                .snapshot_attribute_names(view, source)?
                .ok_or(VfsError::NotFound)?
            {
                if key == super::snapshot_restore::STAGE_ATTRIBUTE {
                    continue;
                }
                let value = self
                    .volume
                    .snapshot_attribute(view, source, &key)?
                    .ok_or(VfsError::NotFound)?
                    .ok_or(VfsError::NotFound)?;
                metadata_bytes = metadata_bytes
                    .checked_add(key.len())
                    .and_then(|n| n.checked_add(value.len()))
                    .ok_or(VfsError::Limit("restore tree metadata overflow"))?;
                if metadata_bytes > limits.max_metadata_bytes {
                    return Err(VfsError::Limit("restore tree metadata budget exhausted"));
                }
                attributes.push((key, value));
            }
            let index = nodes.len();
            if metadata.object_type == ObjectType::Directory {
                let mut cursor = None;
                loop {
                    let page = self
                        .volume
                        .snapshot_read_directory_page(view, source, cursor, 64)?;
                    for entry in page.entries {
                        if nodes.len() + pending.len() + 1 >= limits.max_nodes {
                            return Err(VfsError::Limit("restore tree node budget exhausted"));
                        }
                        let name = String::from_utf8(entry.name).map_err(|_| VfsError::Invalid)?;
                        let child_depth = depth.checked_add(1).ok_or(VfsError::Invalid)?;
                        if child_depth > limits.max_depth {
                            return Err(VfsError::Limit("restore tree depth budget exhausted"));
                        }
                        pending.push((entry.child_id, Some(index), name, child_depth));
                    }
                    if page.eof {
                        break;
                    }
                    cursor = Some(page.next);
                }
            } else {
                // The child file copier's complete preservation admission is
                // checked here too, before even the staging root is created.
                let block = self.volume.ident().geometry().block_size as u64;
                let rounded = metadata
                    .size_bytes
                    .checked_add(block - 1)
                    .ok_or(VfsError::Invalid)?
                    / block
                    * block;
                let mut cursor = 0;
                let mut records = 0usize;
                loop {
                    let page = self
                        .volume
                        .snapshot_allocation_page(view, source, cursor, 64)?;
                    records = records
                        .checked_add(page.ranges.len())
                        .ok_or(VfsError::Invalid)?;
                    if records > 4096 {
                        return Err(VfsError::Limit(
                            "restore file allocation inventory exceeds 4096 ranges",
                        ));
                    }
                    for range in &page.ranges {
                        let end = range
                            .offset
                            .checked_add(range.length)
                            .ok_or(VfsError::Invalid)?;
                        if !range.unwritten && end > rounded {
                            return Err(VfsError::NotSupported);
                        }
                    }
                    if page.eof {
                        break;
                    }
                    if page.next <= cursor {
                        return Err(VfsError::Corrupt(
                            "snapshot allocation cursor did not advance".into(),
                        ));
                    }
                    cursor = page.next;
                }
                // snapshot_restore_file has a bounded opaque payload admission.
                let own_bytes = comment.len()
                    + attributes
                        .iter()
                        .map(|(k, v)| k.len() + v.len())
                        .sum::<usize>();
                if own_bytes > 4 * 1024 * 1024 {
                    return Err(VfsError::Limit("restore file metadata exceeds 4 MiB"));
                }
            }
            nodes.push(Node {
                source,
                parent,
                name,
                metadata,
                comment,
                attributes,
            });
        }
        Ok(nodes)
    }

    fn verify_restored_node(&mut self, node: &Node, object: ObjectId) -> Result<(), VfsError> {
        let actual = self
            .volume
            .visible_metadata(object)?
            .ok_or(VfsError::NotFound)?;
        if actual.object_type != node.metadata.object_type
            || actual.size_bytes != node.metadata.size_bytes
            || actual.link_count != node.metadata.link_count
            || PreservedMetadata::from(actual) != PreservedMetadata::from(node.metadata)
            || self.volume.object_comment(object)? != node.comment
            || self
                .volume
                .attribute_names(object)?
                .into_iter()
                .filter(|k| k != super::snapshot_restore::STAGE_ATTRIBUTE)
                .collect::<Vec<_>>()
                != node
                    .attributes
                    .iter()
                    .map(|(k, _)| k.clone())
                    .collect::<Vec<_>>()
        {
            return Err(VfsError::Corrupt(
                "restored tree metadata verification failed".into(),
            ));
        }
        for (key, value) in &node.attributes {
            if self.volume.attribute(object, key)?.as_ref() != Some(value) {
                return Err(VfsError::Corrupt(
                    "restored tree attribute verification failed".into(),
                ));
            }
        }
        Ok(())
    }

    fn remove_staged_tree(
        &mut self,
        root: ObjectId,
        parent: ObjectId,
        stage: &str,
        now: Timespec,
    ) -> Result<(), CoreError> {
        // Enumerate live staging, not just successfully returned child IDs:
        // a child publication error may leave its own named stage behind.
        let mut pending = vec![(parent, stage.to_owned(), root, false)];
        while let Some((parent, name, object, visited)) = pending.pop() {
            let stat = self
                .volume
                .visible_metadata(object)?
                .ok_or(CoreError::NotFound)?;
            if stat.object_type == ObjectType::Directory && !visited {
                pending.push((parent, name, object, true));
                let mut cursor = None;
                loop {
                    let page = self.volume.read_directory_page(object, cursor, 64)?;
                    for entry in page.entries {
                        let name = String::from_utf8(entry.name)
                            .map_err(|_| CoreError::InvalidMetadata("invalid staging name"))?;
                        pending.push((object, name, entry.child_id, false));
                    }
                    if page.eof {
                        break;
                    }
                    cursor = Some(page.next);
                }
            } else if stat.object_type == ObjectType::Directory {
                self.volume.remove_directory(parent, &name, now)?;
            } else {
                self.volume.delete_file(parent, &name, now)?;
            }
        }
        self.volume.sync()
    }

    pub fn snapshot_restore_tree(
        &mut self,
        reader: Handle,
        source: ObjectId,
        parent: ObjectId,
        name: &str,
        limits: TreeRestoreLimits,
        now: Timespec,
    ) -> Result<ObjectId, VfsError> {
        if limits.max_nodes == 0 || limits.max_depth == 0 || limits.max_metadata_bytes == 0 {
            return Err(VfsError::Invalid);
        }
        if self.mount_mode() != MountMode::ReadWrite {
            return Err(VfsError::ReadOnly);
        }
        let view = self
            .snapshot_readers
            .get(&reader)
            .ok_or(VfsError::Stale)?
            .clone();
        if self.volume.lookup_in_directory(parent, name)?.is_some() {
            return Err(VfsError::AlreadyExists);
        }
        let nodes = self.restore_tree_plan(&view, source, limits)?;
        self.sync_filesystem()?;
        let target_key =
            afsplus_core::name_key::comparison_key(self.volume.ident(), name.as_bytes())?;
        let stage = (0..256)
            .map(|nonce| format!(".afsplus-restore-{reader:016x}-{source:016x}-{nonce:02x}"))
            .find_map(|stage| {
                match afsplus_core::name_key::comparison_key(self.volume.ident(), stage.as_bytes())
                {
                    Ok(key) if key == target_key => return None,
                    Ok(_) => {}
                    Err(e) => return Some(Err(e)),
                }
                match self.volume.lookup_in_directory(parent, &stage) {
                    Ok(None) => Some(Ok(stage)),
                    Ok(Some(_)) => None,
                    Err(e) => Some(Err(e)),
                }
            })
            .ok_or(VfsError::AlreadyExists)??;
        let root=match self.volume.create_directory(parent,&stage,now) {
            Ok(root)=>root,
            Err(e)=>return Err(VfsError::Corrupt(format!("restore tree stage {stage}: creation failed: {e}; inspect destination before retry"))),
        };
        let mut objects = vec![root];
        let result = (|| -> Result<(), VfsError> {
            self.volume.set_attributes(
                root,
                &[(
                    super::snapshot_restore::STAGE_ATTRIBUTE,
                    Some(root.to_le_bytes().as_slice()),
                )],
                AttributeWriteMode::Create,
                now,
            )?;
            for node in nodes.iter().skip(1) {
                let parent = objects[node.parent.ok_or(VfsError::Invalid)?];
                let object = if node.metadata.object_type == ObjectType::Directory {
                    let object = self.volume.create_directory(parent, &node.name, now)?;
                    self.volume.set_attributes(
                        object,
                        &[(
                            super::snapshot_restore::STAGE_ATTRIBUTE,
                            Some(root.to_le_bytes().as_slice()),
                        )],
                        AttributeWriteMode::Create,
                        now,
                    )?;
                    object
                } else {
                    self.restore_file_private(
                        reader,
                        node.source,
                        parent,
                        &node.name,
                        now,
                        Some(root),
                    )?
                };
                objects.push(object);
            }
            // Child creation changes parents. Restore directory metadata last.
            for (index, node) in nodes.iter().enumerate().rev() {
                if node.metadata.object_type == ObjectType::Directory {
                    let object = objects[index];
                    if !node.attributes.is_empty() {
                        let changes: Vec<_> = node
                            .attributes
                            .iter()
                            .map(|(k, v)| (k.as_str(), Some(v.as_slice())))
                            .collect();
                        self.volume.set_attributes(
                            object,
                            &changes,
                            AttributeWriteMode::Create,
                            now,
                        )?;
                    }
                    self.volume.set_object_comment(object, &node.comment, now)?;
                    self.volume
                        .restore_object_metadata(object, PreservedMetadata::from(node.metadata))?;
                }
                self.verify_restored_node(node, objects[index])?;
            }
            // Build child inventories once rather than rescan the whole plan
            // for each directory. Memory and traversal stay proportional to N.
            let mut inventories = vec![BTreeMap::new(); nodes.len()];
            for (index, node) in nodes.iter().enumerate().skip(1) {
                inventories[node.parent.ok_or(VfsError::Invalid)?]
                    .insert(node.name.as_bytes().to_vec(), objects[index]);
            }
            // Verify every name and ensure each directory contains exactly its
            // captured children, including empty directories and no stages.
            for (index, node) in nodes.iter().enumerate() {
                if node.metadata.object_type != ObjectType::Directory {
                    continue;
                }
                let expected = &inventories[index];
                let mut actual = BTreeMap::new();
                let mut cursor = None;
                loop {
                    let page = self
                        .volume
                        .read_directory_page(objects[index], cursor, 64)?;
                    for entry in page.entries {
                        actual.insert(entry.name, entry.child_id);
                    }
                    if page.eof {
                        break;
                    }
                    cursor = Some(page.next);
                }
                if &actual != expected {
                    return Err(VfsError::Corrupt(
                        "restored tree namespace verification failed".into(),
                    ));
                }
            }
            self.volume.sync()?;
            self.volume
                .publish_restored_object(parent, &stage, name, now)?;
            self.volume.sync()?;
            Ok(())
        })();
        if let Err(error) = result {
            match self.volume.lookup_in_directory(parent, &stage) {
                Ok(Some(object)) if object == root => {
                    if let Err(cleanup) = self.remove_staged_tree(root, parent, &stage, now) {
                        return Err(VfsError::Corrupt(format!(
                            "restore tree failed: {error}; retained stage {stage}: {cleanup}"
                        )));
                    }
                }
                Ok(None) => {}
                other => {
                    return Err(VfsError::Corrupt(format!(
                        "restore tree failed: {error}; inspect stage {stage}: {other:?}"
                    )))
                }
            }
            return Err(error);
        }
        Ok(root)
    }
}
