//! Bounded semantic inspection of a retained view, independent of live labels.
use afsplus_block::BlockDevice;
use afsplus_core::{
    volume::{FileAllocationRange, ObjectMetadata, SnapshotHandle, SnapshotInfo},
    Volume,
};
use afsplus_format::{object::ObjectType, OBJECT_ROOT};
use std::collections::{BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub bytes: usize,
    pub ranges: usize,
    pub page_entries: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Entry {
    pub path: Vec<String>,
    pub metadata: ObjectMetadata,
    /// File contents or opaque symlink target; empty for a directory.
    pub data: Vec<u8>,
    pub allocation: Vec<FileAllocationRange>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct View {
    pub info: SnapshotInfo,
    pub root: ObjectMetadata,
    pub entries: Vec<Entry>,
}

fn validate_limits(limits: Limits) -> Result<(), String> {
    if limits.entries > 1024
        || limits.bytes > 64 * 1024 * 1024
        || limits.ranges > 4096
        || !(1..=64).contains(&limits.page_entries)
    {
        return Err("captured inspection limits".into());
    }
    Ok(())
}

pub fn inspect<D: BlockDevice>(
    volume: &mut Volume<D>,
    handle: &SnapshotHandle,
    limits: Limits,
) -> Result<View, String> {
    validate_limits(limits)?;
    let root = volume
        .snapshot_stat(handle, OBJECT_ROOT)
        .map_err(|e| e.to_string())?
        .ok_or("captured root missing")?;
    if root.object_type != ObjectType::Directory {
        return Err("captured root type".into());
    }
    let mut queue = VecDeque::from([(OBJECT_ROOT, Vec::<String>::new())]);
    let mut directories = BTreeSet::from([OBJECT_ROOT]);
    let mut paths = BTreeSet::new();
    let mut entries = Vec::new();
    let mut bytes = 0usize;
    let mut ranges = 0usize;
    while let Some((directory, parent)) = queue.pop_front() {
        let mut cursor = None;
        loop {
            let page = volume
                .snapshot_read_directory_page(handle, directory, cursor, limits.page_entries)
                .map_err(|e| e.to_string())?;
            if page.entries.is_empty() && !page.eof {
                return Err("captured directory made no progress".into());
            }
            for child in page.entries {
                if entries.len() >= limits.entries || parent.len() >= 64 {
                    return Err("captured namespace limit".into());
                }
                let name = String::from_utf8(child.name).map_err(|_| "captured name encoding")?;
                if volume
                    .snapshot_lookup(handle, directory, &name)
                    .map_err(|e| e.to_string())?
                    != Some(child.child_id)
                {
                    return Err("captured lookup differs from enumeration".into());
                }
                let mut path = parent.clone();
                path.push(name);
                if !paths.insert(path.clone()) {
                    return Err("duplicate captured path".into());
                }
                let metadata = volume
                    .snapshot_stat(handle, child.child_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("captured child missing")?;
                let mut data = Vec::new();
                let mut allocation = Vec::new();
                match metadata.object_type {
                    ObjectType::Internal => {
                        return Err("internal object in captured namespace".into())
                    }
                    ObjectType::Directory => {
                        if !directories.insert(child.child_id) {
                            return Err("repeated captured directory".into());
                        }
                        queue.push_back((child.child_id, path.clone()));
                    }
                    ObjectType::File | ObjectType::Symlink => {
                        let size =
                            usize::try_from(metadata.size_bytes).map_err(|_| "captured size")?;
                        if size > limits.bytes.saturating_sub(bytes) {
                            return Err("captured content limit".into());
                        }
                        bytes += size;
                        data.try_reserve_exact(size)
                            .map_err(|_| "captured content allocation")?;
                        data.resize(size, 0);
                        if metadata.object_type == ObjectType::Symlink {
                            let n = volume
                                .snapshot_read_link(handle, child.child_id, &mut data)
                                .map_err(|e| e.to_string())?;
                            if n != size {
                                return Err("captured symlink length".into());
                            }
                        } else {
                            let mut offset = 0;
                            while offset < size {
                                let end = size.min(offset + 4096);
                                let n = volume
                                    .snapshot_read_file_at(
                                        handle,
                                        child.child_id,
                                        offset as u64,
                                        &mut data[offset..end],
                                    )
                                    .map_err(|e| e.to_string())?;
                                if n == 0 || n > end - offset {
                                    return Err("captured read made no progress".into());
                                }
                                offset += n;
                            }
                            let mut beyond = [0u8; 1];
                            if volume
                                .snapshot_read_file_at(
                                    handle,
                                    child.child_id,
                                    size as u64,
                                    &mut beyond,
                                )
                                .map_err(|e| e.to_string())?
                                != 0
                            {
                                return Err("captured EOF mismatch".into());
                            }
                            let mut ordinal = 0;
                            loop {
                                let page = volume
                                    .snapshot_allocation_page(
                                        handle,
                                        child.child_id,
                                        ordinal,
                                        limits.page_entries,
                                    )
                                    .map_err(|e| e.to_string())?;
                                if page.ranges.len() > limits.ranges.saturating_sub(ranges) {
                                    return Err("captured allocation limit".into());
                                }
                                if !page.eof && (page.ranges.is_empty() || page.next <= ordinal) {
                                    return Err("captured allocation made no progress".into());
                                }
                                ranges += page.ranges.len();
                                allocation.extend(page.ranges);
                                if page.eof {
                                    break;
                                }
                                ordinal = page.next;
                            }
                        }
                    }
                }
                entries.push(Entry {
                    path,
                    metadata,
                    data,
                    allocation,
                });
            }
            if page.eof {
                let end = volume
                    .snapshot_read_directory_page(
                        handle,
                        directory,
                        Some(page.next),
                        limits.page_entries,
                    )
                    .map_err(|e| e.to_string())?;
                if !end.eof || !end.entries.is_empty() {
                    return Err("captured directory EOF mismatch".into());
                }
                break;
            }
            cursor = Some(page.next);
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(View {
        info: handle.info(),
        root,
        entries,
    })
}

/// Enumerate every registered view with aggregate, not per-view, output budgets.
pub fn inspect_all<D: BlockDevice>(
    volume: &mut Volume<D>,
    max_views: usize,
    mut limits: Limits,
) -> Result<Vec<View>, String> {
    validate_limits(limits)?;
    if max_views == 0 || max_views > 16 {
        return Err("captured registry limit".into());
    }
    let mut views = Vec::new();
    let mut low = 0;
    let mut previous = 0;
    loop {
        let page = volume.snapshot_list(low, 2).map_err(|e| e.to_string())?;
        for info in page.entries {
            if views.len() >= max_views || info.id <= previous {
                return Err("captured registry count or ordering".into());
            }
            previous = info.id;
            let handle = volume.snapshot_open(info.id).map_err(|e| e.to_string())?;
            if handle.info() != info {
                return Err("captured registry identity changed".into());
            }
            let view = inspect(volume, &handle, limits)?;
            limits.entries -= view.entries.len();
            for entry in &view.entries {
                limits.bytes -= entry.data.len();
                limits.ranges -= entry.allocation.len();
            }
            views.push(view);
        }
        match page.next_id {
            None => break,
            Some(next) if next > low && next > previous => low = next,
            Some(_) => return Err("captured registry made no progress".into()),
        }
    }
    Ok(views)
}
