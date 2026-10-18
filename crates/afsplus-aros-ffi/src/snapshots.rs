//! C snapshot operations; native packet callers pass host authorization first.
use super::*;
fn snapshot_kind(kind: afsplus_vfs::NodeKind) -> Result<u32, ArosError> {
    match kind {
        afsplus_vfs::NodeKind::File => Ok(AFSPLUS_AROS_KIND_FILE),
        afsplus_vfs::NodeKind::Directory => Ok(AFSPLUS_AROS_KIND_DIRECTORY),
        afsplus_vfs::NodeKind::Symlink => Ok(AFSPLUS_AROS_KIND_SYMLINK),
        afsplus_vfs::NodeKind::Internal => Err(ArosError::ObjectWrongType),
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct AfsplusArosSnapshotInfo {
    pub id: u64,
    pub generation: u64,
    pub committed_tx_id: u64,
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_create(
    fs: *mut AfsplusAros,
    seconds: i64,
    nanos: u32,
    out: *mut u64,
) -> i32 {
    bridge_status(fs, || {
        require_output(out)?;
        if nanos >= 1_000_000_000 {
            return Err(ArosError::BadNumber);
        }
        let id = bridge_mut(fs)?.adapter.snapshot_create(Timespec {
            seconds,
            nanoseconds: nanos,
        })?;
        write_output(out, id)
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_delete(
    fs: *mut AfsplusAros,
    id: u64,
    seconds: i64,
    nanos: u32,
) -> i32 {
    bridge_status(fs, || {
        if nanos >= 1_000_000_000 {
            return Err(ArosError::BadNumber);
        }
        bridge_mut(fs)?.adapter.snapshot_delete(
            id,
            Timespec {
                seconds,
                nanoseconds: nanos,
            },
        )
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_list(
    fs: *mut AfsplusAros,
    low_id: u64,
    buffer: *mut u8,
    capacity: u32,
    limit: u32,
    count: *mut u32,
    next: *mut u64,
) -> i32 {
    bridge_status(fs, || {
        require_output(count)?;
        require_output(next)?;
        let limit = limit.min(capacity / 24).min(64);
        if limit == 0 {
            return Err(ArosError::BadNumber);
        }
        let dst = output_slice(buffer, capacity)?;
        let page = bridge_mut(fs)?
            .adapter
            .snapshot_list(low_id, limit as usize)?;
        for (index, info) in page.entries.iter().enumerate() {
            let record = AfsplusArosSnapshotInfo {
                id: info.id,
                generation: info.generation,
                committed_tx_id: info.committed_tx_id,
            };
            // SAFETY: output_slice validated capacity; each slot contains 24 bytes.
            unsafe {
                ptr::copy_nonoverlapping(
                    ptr::from_ref(&record).cast::<u8>(),
                    dst[index * 24..].as_mut_ptr(),
                    24,
                );
            }
        }
        write_output(count, page.entries.len() as u32)?;
        write_output(next, page.next_id.unwrap_or(0))
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_open(fs: *mut AfsplusAros, id: u64, out: *mut u64) -> i32 {
    bridge_status(fs, || {
        require_output(out)?;
        let reader = bridge_mut(fs)?.adapter.snapshot_open(id)?;
        write_output(out, reader)
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_close(fs: *mut AfsplusAros, reader: u64) -> i32 {
    bridge_status(fs, || bridge_mut(fs)?.adapter.snapshot_close(reader))
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_lookup(
    fs: *mut AfsplusAros,
    reader: u64,
    directory: u64,
    name: *const u8,
    length: u32,
    out: *mut u64,
) -> i32 {
    bridge_status(fs, || {
        require_output(out)?;
        if length > 255 {
            return Err(ArosError::InvalidComponentName);
        }
        let id = bridge_mut(fs)?.adapter.snapshot_lookup(
            reader,
            directory,
            input_bytes(name, length)?,
        )?;
        write_output(out, id)
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_read(
    fs: *mut AfsplusAros,
    reader: u64,
    object: u64,
    offset: u64,
    buffer: *mut u8,
    capacity: u32,
    count: *mut u32,
) -> i32 {
    bridge_status(fs, || {
        require_output(count)?;
        let count_read = bridge_mut(fs)?.adapter.snapshot_read(
            reader,
            object,
            offset,
            output_slice(buffer, capacity)?,
        )?;
        write_output(count, count_read as u32)
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_stat(
    fs: *mut AfsplusAros,
    reader: u64,
    object: u64,
    output: *mut AfsplusArosStat,
) -> i32 {
    bridge_status(fs, || {
        let s = bridge_mut(fs)?.adapter.snapshot_stat(reader, object)?;
        write_sized_output(
            output,
            AFSPLUS_AROS_STAT_FIRST_LAYOUT,
            AfsplusArosStat {
                struct_size: 0,
                kind: snapshot_kind(s.kind)?,
                object_id: s.object_id,
                size: s.size,
                allocated_size: s.allocated_size,
                protection: s.protection,
                links: s.links,
                created_seconds: s.created.seconds,
                modified_seconds: s.modified.seconds,
                changed_seconds: s.changed.seconds,
                created_nanoseconds: s.created.nanoseconds,
                modified_nanoseconds: s.modified.nanoseconds,
                changed_nanoseconds: s.changed.nanoseconds,
                owner_uid: s.owner_uid,
                owner_gid: s.owner_gid,
                ..Default::default()
            },
            |value, size| value.struct_size = size,
        )
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_dir_open(
    fs: *mut AfsplusAros,
    reader: u64,
    object: u64,
    out: *mut u64,
) -> i32 {
    bridge_status(fs, || {
        require_output(out)?;
        let dir = bridge_mut(fs)?
            .adapter
            .snapshot_directory_open(reader, object)?;
        write_output(out, dir)
    })
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_dir_close(fs: *mut AfsplusAros, dir: u64) -> i32 {
    bridge_status(fs, || bridge_mut(fs)?.adapter.snapshot_directory_close(dir))
}
#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_dir_read(
    fs: *mut AfsplusAros,
    dir: u64,
    buffer: *mut u8,
    capacity: u32,
    limit: u32,
    count: *mut u32,
    eof: *mut u32,
) -> i32 {
    bridge_status(fs, || {
        require_output(count)?;
        require_output(eof)?;
        let limit = limit.min(capacity / AFSPLUS_AROS_DIR_RECORD_MAX).min(64);
        if limit == 0 {
            return Err(ArosError::BadNumber);
        }
        let dst = output_slice(buffer, capacity)?;
        let page = bridge_mut(fs)?
            .adapter
            .snapshot_directory_read(dir, limit as usize)?;
        let header = std::mem::size_of::<AfsplusArosDirEntry>();
        let mut at = 0;
        for entry in &page.entries {
            let length = (header + entry.name.len() + 7) & !7;
            let fixed = AfsplusArosDirEntry {
                object_id: entry.object_id,
                kind: snapshot_kind(entry.kind)?,
                name_length: entry.name.len() as u32,
                record_length: length as u32,
                reserved: 0,
            };
            let slot = dst
                .get_mut(at..at + length)
                .ok_or(ArosError::ObjectTooLarge)?;
            slot.fill(0);
            // SAFETY: complete fixed-size record fits the checked slot, copied bytewise.
            unsafe {
                ptr::copy_nonoverlapping(
                    ptr::from_ref(&fixed).cast::<u8>(),
                    slot.as_mut_ptr(),
                    header,
                );
            }
            slot[header..header + entry.name.len()].copy_from_slice(&entry.name);
            at += length;
        }
        write_output(count, page.entries.len() as u32)?;
        write_output(eof, u32::from(page.eof))
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_restore_file(
    fs: *mut AfsplusAros,
    reader: u64,
    source: u64,
    destination: u64,
    name: *const u8,
    length: u32,
    seconds: i64,
    nanos: u32,
    out: *mut u64,
) -> i32 {
    bridge_status(fs, || {
        require_output(out)?;
        if length > 255 {
            return Err(ArosError::InvalidComponentName);
        }
        let id = bridge_mut(fs)?.adapter.snapshot_restore_file(
            reader,
            source,
            optional_lock(destination),
            input_bytes(name, length)?,
            timestamp(seconds, nanos)?,
        )?;
        write_output(out, id)
    })
}

#[no_mangle]
pub extern "C" fn afsplus_aros_snapshot_restore_tree(
    fs: *mut AfsplusAros,
    reader: u64,
    source: u64,
    destination: u64,
    name: *const u8,
    length: u32,
    max_nodes: u32,
    max_depth: u32,
    max_metadata_bytes: u32,
    seconds: i64,
    nanos: u32,
    out: *mut u64,
) -> i32 {
    bridge_status(fs, || {
        require_output(out)?;
        if length > 255 || max_nodes == 0 || max_depth == 0 || max_metadata_bytes == 0 {
            return Err(ArosError::BadNumber);
        }
        let id = bridge_mut(fs)?.adapter.snapshot_restore_tree(
            reader,
            source,
            optional_lock(destination),
            input_bytes(name, length)?,
            afsplus_vfs::TreeRestoreLimits {
                max_nodes: max_nodes as usize,
                max_depth: max_depth as usize,
                max_metadata_bytes: max_metadata_bytes as usize,
            },
            timestamp(seconds, nanos)?,
        )?;
        write_output(out, id)
    })
}
