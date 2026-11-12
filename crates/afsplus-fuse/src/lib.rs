//! Host-neutral FUSE protocol adapter for AFS+.
//!
//! [`FuseAdapter`] contains the semantics that can be tested without a FUSE
//! kernel driver. The optional `fuser-adapter` feature only translates fuser
//! request/reply types around this layer.

use std::collections::BTreeMap;

use afsplus_block::BlockDevice;
use afsplus_core::MountMode;
use afsplus_format::attrs::ATTRIBUTE_VALUE_MAX_BYTES;
use afsplus_format::posix;
use afsplus_format::{Timespec, OBJECT_ROOT};
pub use afsplus_vfs::AttributeWriteMode;
use afsplus_vfs::{
    AccessMode, Durability, Handle, NodeKind, ObjectId, Stat, StatFs, Vfs, VfsError,
};

pub mod diagnostics;
#[cfg(feature = "fuser-adapter")]
pub mod fuser_adapter;

const FIRST_REAL_DIRECTORY_OFFSET: u64 = 3;

/// Names a host mount presents for a mounted volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostNames {
    /// Source name of the mount (`FSName`).
    pub filesystem: String,
    /// Volume name shown by the host.
    pub volume: String,
}

/// The host names of a mounted volume, from its committed label (ADR-104).
/// The identification block keeps the label given at format time, which a
/// relabel makes stale, so a mount never names itself from that block.
pub fn host_names<D: BlockDevice>(vfs: &Vfs<D>) -> HostNames {
    let label = vfs.volume_label();
    HostNames {
        filesystem: format!("afsplus: {label}"),
        volume: label.to_owned(),
    }
}

/// How the host spells attribute names. The volume stores names with a
/// namespace (`user.`, `system.`, `security.`, `aros.`); every stored name has
/// exactly one host spelling and every accepted host name exactly one stored
/// name, so nothing is hidden and nothing collides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostAttributeNames {
    /// Linux requires a namespace of its own. `user.` and `security.` names
    /// are stored as they are; the stored `system.` and `aros.` namespaces
    /// appear under `trusted.afsplus.`, which needs privilege on the host.
    /// A Linux `system.` or other `trusted.` name is refused: the kernel
    /// attaches meaning to them that this volume does not implement.
    Linux,
    /// macOS names are free-form (`com.apple.quarantine`), so a host name is
    /// stored under `user.`. The other stored namespaces, and a stored user
    /// name that itself begins with `afsplus.`, appear as `afsplus.` followed
    /// by the stored name.
    MacOs,
}

impl HostAttributeNames {
    /// The convention of the host this was compiled for.
    pub const fn native() -> Self {
        if cfg!(target_os = "macos") {
            HostAttributeNames::MacOs
        } else {
            HostAttributeNames::Linux
        }
    }

    /// The stored name of a host name; [`VfsError::NotSupported`] for a name
    /// this convention does not carry.
    pub fn stored(self, host: &[u8]) -> Result<String, VfsError> {
        let host = utf8_name(host)?;
        match self {
            HostAttributeNames::Linux => {
                if host.starts_with("user.") || host.starts_with("security.") {
                    Ok(host.to_owned())
                } else if let Some(rest) = host.strip_prefix("trusted.afsplus.") {
                    if rest.starts_with("system.") || rest.starts_with("aros.") {
                        Ok(rest.to_owned())
                    } else {
                        Err(VfsError::NotSupported)
                    }
                } else {
                    Err(VfsError::NotSupported)
                }
            }
            HostAttributeNames::MacOs => match host.strip_prefix("afsplus.") {
                // The user namespace has its plain spelling unless the rest
                // would read as this escape again.
                Some(rest) if rest.starts_with("user.") && !rest.starts_with("user.afsplus.") => {
                    Err(VfsError::NotSupported)
                }
                Some(rest) => Ok(rest.to_owned()),
                None => Ok(format!("user.{host}")),
            },
        }
    }

    /// The host names of the object's comment and protection word, which
    /// are fields of the object record, not stored attributes (ADR-120).
    pub const fn field_names(self) -> [(&'static str, ObjectField); 2] {
        match self {
            HostAttributeNames::Linux => [
                ("user.afsplus.aros.comment", ObjectField::Comment),
                ("user.afsplus.aros.protection", ObjectField::Protection),
            ],
            HostAttributeNames::MacOs => [
                ("afsplus.aros.comment", ObjectField::Comment),
                ("afsplus.aros.protection", ObjectField::Protection),
            ],
        }
    }

    /// The object field a host name addresses, if it addresses one.
    pub fn field(self, host: &[u8]) -> Option<ObjectField> {
        self.field_names()
            .into_iter()
            .find(|(name, _)| name.as_bytes() == host)
            .map(|(_, field)| field)
    }

    /// The one host spelling of a stored name.
    pub fn host(self, stored: &str) -> String {
        match self {
            HostAttributeNames::Linux => {
                if stored.starts_with("user.") || stored.starts_with("security.") {
                    stored.to_owned()
                } else {
                    format!("trusted.afsplus.{stored}")
                }
            }
            HostAttributeNames::MacOs => match stored.strip_prefix("user.") {
                Some(rest) if !rest.starts_with("afsplus.") => rest.to_owned(),
                _ => format!("afsplus.{stored}"),
            },
        }
    }
}

/// A field of the object record a host reads and writes as an attribute.
/// The comment is its UTF-8 text and absent when empty; the protection word
/// is always present, as `0x` and eight upper-case hex digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectField {
    Comment,
    Protection,
}

/// The protection word a host wrote: `0x` optional, then one to sixteen hex
/// digits, nothing else. A value beyond 32 bits names bits the word does not
/// have, and is refused like malformed text.
pub fn parse_protection(value: &[u8]) -> Option<u32> {
    let digits = value
        .strip_prefix(b"0x")
        .or_else(|| value.strip_prefix(b"0X"))
        .unwrap_or(value);
    if digits.is_empty() || digits.len() > 16 || !digits.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let word = u64::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()?;
    u32::try_from(word).ok()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FuseConfig {
    pub uid: u32,
    pub gid: u32,
    pub file_mode: u16,
    pub directory_mode: u16,
    /// Make every successful data mutation durable before replying.
    ///
    /// This is a transport workaround for hosts that flush dirty pages through
    /// `WRITE`/`SETATTR` but return from `fsync(2)` without sending FUSE
    /// `FSYNC`. It deliberately strengthens normal FUSE writeback semantics.
    pub durable_data_replies: bool,
    pub attribute_names: HostAttributeNames,
    /// Read a `SETXATTR` with an empty value as "make this attribute absent".
    ///
    /// A transport workaround like `durable_data_replies`: macFUSE's FSKit
    /// backend never sends `REMOVEXATTR`; `removexattr(2)` arrives as a
    /// `SETXATTR` with no bytes and no flags, so a removal and an empty
    /// value are one request. With this set the removal works and an empty
    /// value cannot be stored through that mount; it stays readable when
    /// another host stored it.
    pub empty_value_removes: bool,
}

impl Default for FuseConfig {
    fn default() -> Self {
        FuseConfig {
            uid: 0,
            gid: 0,
            file_mode: 0o644,
            directory_mode: 0o755,
            durable_data_replies: false,
            attribute_names: HostAttributeNames::native(),
            empty_value_removes: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuseAttributes {
    pub object_id: ObjectId,
    pub kind: NodeKind,
    pub size: u64,
    /// Allocated storage in POSIX 512-byte units.
    pub blocks_512: u64,
    pub links: u32,
    pub mode: u16,
    pub uid: u32,
    pub gid: u32,
    pub created: Timespec,
    pub modified: Timespec,
    pub changed: Timespec,
    pub block_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuseDirectoryEntry {
    pub name: Vec<u8>,
    pub object_id: ObjectId,
    pub kind: NodeKind,
    /// Opaque value supplied back by the kernel to resume after this entry.
    pub next_offset: u64,
}

/// Stateful translation from FUSE operations to the portable AFS+ VFS API.
///
/// Object IDs are stable and map one-to-one to FUSE inode numbers. Directory
/// parents are session metadata used only to synthesize `..`; they never alter
/// the on-disk namespace.
pub struct FuseAdapter<D: BlockDevice> {
    vfs: Vfs<D>,
    config: FuseConfig,
    parents: BTreeMap<ObjectId, ObjectId>,
    /// Handles on symlinks themselves, see [`FuseAdapter::open_file`]. They
    /// hold no VFS handle and are numbered from [`SYMLINK_HANDLE_BASE`], a
    /// range the VFS's counter, which starts at one, never reaches.
    symlink_handles: BTreeMap<Handle, ObjectId>,
    next_symlink_handle: Handle,
}

/// The first handle number of a handle on a symlink.
const SYMLINK_HANDLE_BASE: Handle = 1 << 63;

impl<D: BlockDevice> FuseAdapter<D> {
    pub fn new(vfs: Vfs<D>, config: FuseConfig) -> Self {
        let mut parents = BTreeMap::new();
        parents.insert(OBJECT_ROOT, OBJECT_ROOT);
        FuseAdapter {
            vfs,
            config,
            parents,
            symlink_handles: BTreeMap::new(),
            next_symlink_handle: SYMLINK_HANDLE_BASE,
        }
    }

    pub fn config(&self) -> FuseConfig {
        self.config
    }

    pub fn root_object(&self) -> ObjectId {
        self.vfs.root_object()
    }

    pub fn lookup(&mut self, parent: ObjectId, name: &[u8]) -> Result<FuseAttributes, VfsError> {
        let object_id = match name {
            b"." => parent,
            b".." => self.parent_of(parent),
            _ => self.vfs.lookup(parent, utf8_name(name)?)?,
        };
        self.parents.entry(object_id).or_insert(parent);
        self.attributes(object_id)
    }

    pub fn attributes(&mut self, object_id: ObjectId) -> Result<FuseAttributes, VfsError> {
        let stat = self.vfs.stat(object_id)?;
        Ok(self.map_attributes(stat))
    }

    /// Writes the POSIX permission bits, keeping the AmigaDOS-only bits of
    /// the protection word. A bit this format cannot carry, sticky, is
    /// refused by the VFS rather than dropped, and the refusal reaches the
    /// caller as EOPNOTSUPP.
    pub fn set_posix_mode(
        &mut self,
        object_id: ObjectId,
        mode: u16,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.vfs.set_posix_mode(object_id, mode, now)
    }

    /// Sets either half of the owner; `None` leaves that half alone.
    pub fn set_owner(
        &mut self,
        object_id: ObjectId,
        uid: Option<u32>,
        gid: Option<u32>,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.vfs.set_owner(object_id, uid, gid, now)
    }

    /// Sets the modification time, as `utimes` does.
    pub fn set_times(
        &mut self,
        object_id: ObjectId,
        modified: Timespec,
        now: Timespec,
    ) -> Result<(), VfsError> {
        self.vfs.set_times(object_id, modified, now)
    }

    pub fn statfs(&self) -> StatFs {
        self.vfs.statfs()
    }

    /// Leave the maintenance an unlink or a last close would do for the host
    /// to run itself; see [`Vfs::set_inline_maintenance`].
    pub fn set_inline_maintenance(&mut self, enabled: bool) {
        self.vfs.set_inline_maintenance(enabled);
    }

    /// One transaction of the maintenance operations leave behind; see
    /// [`Vfs::maintenance_step`]. True while more remains.
    ///
    /// Not on the call that reports free space, where it was first put: a
    /// file browser asks that many times a second. Not inside unlink or close
    /// either, because the macOS driver answers one request at a time and
    /// every other program waits behind whatever a request does. The driver
    /// calls this from a thread of its own.
    ///
    /// On a delayed mount (ADR-121) this is also the clock that commits the
    /// window once the volume has been idle for a second or its oldest
    /// change reaches the maximum age; deleted files are cleaned only when
    /// no window is open, so that cleanup does not commit a window early.
    pub fn maintenance_step(&mut self, now: Timespec) -> bool {
        if matches!(self.vfs.durability(), Durability::Delayed { .. }) {
            // A failed commit has already been reported to nobody: the next
            // request that needs the window answers the error.
            let _ = self.vfs.commit_if_due(now);
            if self.vfs.changes_pending() {
                return false;
            }
        }
        self.vfs.maintenance_step(now).unwrap_or(false)
    }

    /// When changes reach the disk; see [`Vfs::set_durability`]. A delayed
    /// mount makes no data write durable before answering it, whatever
    /// [`FuseConfig::durable_data_replies`] says: the window's commit, an
    /// fsync that reaches the driver and the unmount are its durability
    /// points.
    pub fn set_durability(&mut self, durability: Durability) -> Result<(), VfsError> {
        self.vfs.set_durability(durability)
    }

    pub fn durability(&self) -> Durability {
        self.vfs.durability()
    }

    /// Opens a file, or a symlink itself.
    ///
    /// macOS opens the link and not its target when a program passes
    /// `O_SYMLINK`, and `copyfile(3)`, which `cp` uses, does exactly that to
    /// copy a link's extended attributes, mode and times through the
    /// descriptor. This answered "Is a directory", the VFS's word for "not a
    /// file", and `cp -R` reported every symlink it copied. A handle on a
    /// symlink carries no data: attribute and metadata requests address the
    /// object by inode and work as on any object, and a read or write through
    /// it is refused with [`VfsError::NotSupported`], as APFS refuses one.
    pub fn open_file(
        &mut self,
        object_id: ObjectId,
        access: AccessMode,
        truncate: bool,
        now: Timespec,
    ) -> Result<Handle, VfsError> {
        if self.vfs.stat(object_id)?.kind == NodeKind::Symlink {
            if truncate {
                return Err(VfsError::Invalid);
            }
            if access != AccessMode::ReadOnly && self.vfs.mount_mode() != MountMode::ReadWrite {
                return Err(VfsError::ReadOnly);
            }
            let handle = self.next_symlink_handle;
            self.next_symlink_handle = handle
                .checked_add(1)
                .ok_or(VfsError::Limit("handle space exhausted"))?;
            self.symlink_handles.insert(handle, object_id);
            return Ok(handle);
        }
        let handle = self.vfs.open_file(object_id, access)?;
        if truncate {
            let result = self
                .vfs
                .truncate(handle, 0, now)
                .and_then(|()| self.finish_data_mutation(handle));
            if let Err(error) = result {
                let _ = self.vfs.close(handle);
                return Err(error);
            }
        }
        Ok(handle)
    }

    pub fn open_directory(&mut self, object_id: ObjectId) -> Result<Handle, VfsError> {
        self.vfs.open_directory(object_id)
    }

    pub fn close(&mut self, handle: Handle) -> Result<(), VfsError> {
        if self.symlink_handles.remove(&handle).is_some() {
            return Ok(());
        }
        self.vfs.close(handle)
    }

    /// Whether `handle` is open on a symlink rather than on a file.
    pub fn is_symlink_handle(&self, handle: Handle) -> bool {
        self.symlink_handles.contains_key(&handle)
    }

    /// A handle on a symlink has no data to read or write.
    fn data_handle(&self, handle: Handle) -> Result<(), VfsError> {
        if self.is_symlink_handle(handle) {
            Err(VfsError::NotSupported)
        } else {
            Ok(())
        }
    }

    pub fn read(&mut self, handle: Handle, offset: u64, size: u32) -> Result<Vec<u8>, VfsError> {
        self.data_handle(handle)?;
        let mut data = vec![0; size as usize];
        let read = self.vfs.read(handle, offset, &mut data)?;
        data.truncate(read);
        Ok(data)
    }

    pub fn write(
        &mut self,
        handle: Handle,
        offset: u64,
        data: &[u8],
        now: Timespec,
    ) -> Result<usize, VfsError> {
        self.data_handle(handle)?;
        let written = self.vfs.write(handle, offset, data, now)?;
        self.finish_data_mutation(handle)?;
        Ok(written)
    }

    pub fn truncate(
        &mut self,
        object_id: ObjectId,
        handle: Option<Handle>,
        size: u64,
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        // macOS announces the size of every file it has written, after the
        // writes; a truncation to the size the file has changes nothing,
        // and doing it anyway rewrote a new file's data a second time.
        let current = self.attributes(object_id)?;
        if current.kind == NodeKind::File && current.size == size {
            return Ok(current);
        }
        if let Some(handle) = handle.filter(|handle| !self.is_symlink_handle(*handle)) {
            self.vfs.truncate(handle, size, now)?;
            self.finish_data_mutation(handle)?;
        } else {
            let temporary = self.vfs.open_file(object_id, AccessMode::WriteOnly)?;
            let result = self
                .vfs
                .truncate(temporary, size, now)
                .and_then(|()| self.finish_data_mutation(temporary));
            let close_result = self.vfs.close(temporary);
            result?;
            close_result?;
        }
        self.attributes(object_id)
    }

    /// `mode` is what the creator asked for, already reduced by its umask by
    /// the kernel. `None` means the caller named none and the mount's
    /// configured default applies.
    pub fn create_file(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        access: AccessMode,
        mode: Option<u16>,
        now: Timespec,
    ) -> Result<(FuseAttributes, Handle), VfsError> {
        let object_id = self.vfs.create_file(parent, utf8_name(name)?, now)?;
        self.parents.insert(object_id, parent);
        self.apply_creation_mode(object_id, mode.unwrap_or(self.config.file_mode), now)?;
        let handle = self.vfs.open_file(object_id, access)?;
        Ok((self.attributes(object_id)?, handle))
    }

    /// Writes the mode an object is born with, and its creator's identity.
    ///
    /// A bit the format cannot carry does not fail the creation, as it does
    /// not on any POSIX filesystem asked for one: the rest of the mode is
    /// written and the missing bit is there for the caller to see absent.
    /// What must NOT happen is leaving the word untouched, because an
    /// untouched word reads as 0o700 and the file would be born with a mode
    /// nobody chose.
    fn apply_creation_mode(
        &mut self,
        object_id: ObjectId,
        mode: u16,
        now: Timespec,
    ) -> Result<(), VfsError> {
        if let Err(VfsError::NotSupported) = self.vfs.set_posix_mode(object_id, mode, now) {
            self.vfs
                .set_posix_mode(object_id, mode & !posix::MODE_STICKY, now)?;
        }
        self.vfs
            .set_owner(object_id, Some(self.config.uid), Some(self.config.gid), now)
    }

    pub fn create_symlink(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        target: &[u8],
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        let object_id =
            self.vfs
                .create_symlink(parent, utf8_name(name)?, utf8_name(target)?, now)?;
        self.parents.insert(object_id, parent);
        self.attributes(object_id)
    }

    /// The exact target bytes of a symlink.
    ///
    /// Asked for the length first: a buffer too short leaves the target unread
    /// and answers with the count it needed, so guessing a size would silently
    /// return nothing.
    pub fn read_link(&mut self, object_id: ObjectId) -> Result<Vec<u8>, VfsError> {
        let required = self.vfs.read_link(object_id, &mut [])?;
        let mut target = vec![0u8; required];
        let written = self.vfs.read_link(object_id, &mut target)?;
        target.truncate(written);
        Ok(target)
    }

    pub fn create_directory(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        mode: Option<u16>,
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        let object_id = self.vfs.create_directory(parent, utf8_name(name)?, now)?;
        self.parents.insert(object_id, parent);
        self.apply_creation_mode(object_id, mode.unwrap_or(self.config.directory_mode), now)?;
        self.attributes(object_id)
    }

    pub fn unlink_file(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        let object_id = self.vfs.lookup(parent, utf8_name(name)?)?;
        self.vfs.unlink_file(parent, utf8_name(name)?, now)?;
        self.parents.remove(&object_id);
        Ok(())
    }

    pub fn remove_directory(
        &mut self,
        parent: ObjectId,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        let object_id = self.vfs.lookup(parent, utf8_name(name)?)?;
        self.vfs.remove_directory(parent, utf8_name(name)?, now)?;
        self.parents.remove(&object_id);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rename(
        &mut self,
        source_parent: ObjectId,
        source_name: &[u8],
        target_parent: ObjectId,
        target_name: &[u8],
        replace: bool,
        now: Timespec,
    ) -> Result<(), VfsError> {
        let source_name = utf8_name(source_name)?;
        let target_name = utf8_name(target_name)?;
        let object_id = self.vfs.lookup(source_parent, source_name)?;
        self.vfs.rename(
            source_parent,
            source_name,
            target_parent,
            target_name,
            replace,
            now,
        )?;
        self.parents.insert(object_id, target_parent);
        Ok(())
    }

    pub fn link_file(
        &mut self,
        object_id: ObjectId,
        target_parent: ObjectId,
        target_name: &[u8],
        now: Timespec,
    ) -> Result<FuseAttributes, VfsError> {
        self.vfs
            .link_file(object_id, target_parent, utf8_name(target_name)?, now)?;
        self.attributes(object_id)
    }

    /// Returns at most `max_entries`, including synthesized dot entries.
    pub fn read_directory(
        &mut self,
        directory: ObjectId,
        handle: Handle,
        offset: u64,
        max_entries: usize,
    ) -> Result<Vec<FuseDirectoryEntry>, VfsError> {
        if max_entries == 0 {
            return Ok(Vec::new());
        }

        let mut entries = Vec::with_capacity(max_entries);
        if offset == 0 {
            entries.push(FuseDirectoryEntry {
                name: b".".to_vec(),
                object_id: directory,
                kind: NodeKind::Directory,
                next_offset: 1,
            });
        }
        if offset <= 1 && entries.len() < max_entries {
            entries.push(FuseDirectoryEntry {
                name: b"..".to_vec(),
                object_id: self.parent_of(directory),
                kind: NodeKind::Directory,
                next_offset: 2,
            });
        }
        if entries.len() == max_entries {
            return Ok(entries);
        }

        let underlying_cookie = offset.saturating_sub(2);
        let page =
            self.vfs
                .read_directory(handle, underlying_cookie, max_entries - entries.len())?;
        for (index, entry) in page.entries.into_iter().enumerate() {
            self.parents.entry(entry.object_id).or_insert(directory);
            entries.push(FuseDirectoryEntry {
                name: entry.name,
                object_id: entry.object_id,
                kind: entry.kind,
                next_offset: FIRST_REAL_DIRECTORY_OFFSET + underlying_cookie + index as u64,
            });
        }
        Ok(entries)
    }

    /// The value of a host-named attribute. An absent attribute and a name
    /// the host convention does not carry are both [`VfsError::NotFound`]:
    /// to a reader they are the same fact.
    pub fn get_attribute(&mut self, object_id: ObjectId, name: &[u8]) -> Result<Vec<u8>, VfsError> {
        if let Some(field) = self.config.attribute_names.field(name) {
            return match field {
                ObjectField::Comment => match self.vfs.comment(object_id)? {
                    comment if comment.is_empty() => Err(VfsError::NotFound),
                    comment => Ok(comment.into_bytes()),
                },
                ObjectField::Protection => {
                    Ok(format!("0x{:08X}", self.vfs.stat(object_id)?.protection).into_bytes())
                }
            };
        }
        let stored = match self.config.attribute_names.stored(name) {
            Ok(stored) => stored,
            Err(VfsError::NotSupported) => return Err(VfsError::NotFound),
            Err(error) => return Err(error),
        };
        self.vfs
            .attribute(object_id, &stored)?
            .ok_or(VfsError::NotFound)
    }

    /// Every attribute name in host spelling, each followed by a NUL byte,
    /// in the volume's byte order of stored names, then the object fields
    /// that are present: the protection word always, the comment when set.
    pub fn list_attributes(&mut self, object_id: ObjectId) -> Result<Vec<u8>, VfsError> {
        let mut list = Vec::new();
        for stored in self.vfs.attribute_names(object_id)? {
            list.extend_from_slice(self.config.attribute_names.host(&stored).as_bytes());
            list.push(0);
        }
        let has_comment = !self.vfs.comment(object_id)?.is_empty();
        for (name, field) in self.config.attribute_names.field_names() {
            if field == ObjectField::Protection || has_comment {
                list.extend_from_slice(name.as_bytes());
                list.push(0);
            }
        }
        Ok(list)
    }

    /// Writes an object field under the rules of an attribute: `Create`
    /// finds a present field existing, `Replace` needs it present. A value
    /// the field cannot take is [`VfsError::Invalid`] and changes nothing.
    fn set_field(
        &mut self,
        object_id: ObjectId,
        field: ObjectField,
        value: &[u8],
        mode: AttributeWriteMode,
        now: Timespec,
    ) -> Result<(), VfsError> {
        if value.is_empty() && self.config.empty_value_removes {
            return match self.remove_field(object_id, field, now) {
                Err(VfsError::NotFound) => self.vfs.stat(object_id).map(|_| ()),
                result => result,
            };
        }
        let present = match field {
            ObjectField::Comment => !self.vfs.comment(object_id)?.is_empty(),
            ObjectField::Protection => {
                self.vfs.stat(object_id)?;
                true
            }
        };
        match (mode, present) {
            (AttributeWriteMode::Create, true) => return Err(VfsError::AlreadyExists),
            (AttributeWriteMode::Replace, false) => return Err(VfsError::NotFound),
            _ => {}
        }
        match field {
            ObjectField::Comment => {
                // The classic API carries a comment as a NUL-terminated string.
                let comment = std::str::from_utf8(value).map_err(|_| VfsError::Invalid)?;
                if comment.contains('\0') {
                    return Err(VfsError::Invalid);
                }
                self.vfs.set_comment(object_id, comment, now)
            }
            ObjectField::Protection => {
                let word = parse_protection(value).ok_or(VfsError::Invalid)?;
                self.vfs.set_protection(object_id, word, now)
            }
        }
    }

    /// Removing the comment clears it; the protection word always exists,
    /// so removing it is [`VfsError::Invalid`].
    fn remove_field(
        &mut self,
        object_id: ObjectId,
        field: ObjectField,
        now: Timespec,
    ) -> Result<(), VfsError> {
        match field {
            ObjectField::Comment => {
                if self.vfs.comment(object_id)?.is_empty() {
                    return Err(VfsError::NotFound);
                }
                self.vfs.set_comment(object_id, "", now)
            }
            ObjectField::Protection => {
                self.vfs.stat(object_id)?;
                Err(VfsError::Invalid)
            }
        }
    }

    /// Writes one attribute. A value beyond the format's bound is
    /// [`VfsError::Limit`], which hosts report as too big, not as invalid.
    pub fn set_attribute(
        &mut self,
        object_id: ObjectId,
        name: &[u8],
        value: &[u8],
        mode: AttributeWriteMode,
        now: Timespec,
    ) -> Result<(), VfsError> {
        if let Some(field) = self.config.attribute_names.field(name) {
            return self.set_field(object_id, field, value, mode, now);
        }
        let stored = self.config.attribute_names.stored(name)?;
        if value.is_empty() && self.config.empty_value_removes {
            // "Absent" is the requested state, so an attribute that is
            // already absent is success, as the host's removal of a name it
            // just listed would be.
            return match self.vfs.set_attributes(
                object_id,
                &[(stored.as_str(), None)],
                AttributeWriteMode::Upsert,
                now,
            ) {
                Err(VfsError::NotFound) => self.vfs.stat(object_id).map(|_| ()),
                result => result,
            };
        }
        if value.len() > ATTRIBUTE_VALUE_MAX_BYTES {
            return Err(VfsError::Limit("attribute value exceeds the format bound"));
        }
        self.vfs
            .set_attributes(object_id, &[(stored.as_str(), Some(value))], mode, now)
    }

    pub fn remove_attribute(
        &mut self,
        object_id: ObjectId,
        name: &[u8],
        now: Timespec,
    ) -> Result<(), VfsError> {
        if let Some(field) = self.config.attribute_names.field(name) {
            return self.remove_field(object_id, field, now);
        }
        let stored = match self.config.attribute_names.stored(name) {
            Ok(stored) => stored,
            Err(VfsError::NotSupported) => return Err(VfsError::NotFound),
            Err(error) => return Err(error),
        };
        self.vfs.set_attributes(
            object_id,
            &[(stored.as_str(), None)],
            AttributeWriteMode::Upsert,
            now,
        )
    }

    pub fn fsync(&mut self, handle: Handle) -> Result<(), VfsError> {
        if self.is_symlink_handle(handle) {
            // What a symlink holds is metadata, which the volume makes
            // durable as a whole.
            return self.vfs.sync_filesystem();
        }
        self.vfs.fsync(handle)
    }

    pub fn sync_filesystem(&mut self) -> Result<(), VfsError> {
        self.vfs.sync_filesystem()
    }

    pub fn into_vfs(self) -> Vfs<D> {
        self.vfs
    }

    fn finish_data_mutation(&mut self, handle: Handle) -> Result<(), VfsError> {
        if self.config.durable_data_replies && self.vfs.durability() == Durability::Sync {
            self.vfs.fsync(handle)
        } else {
            Ok(())
        }
    }

    fn parent_of(&self, object_id: ObjectId) -> ObjectId {
        self.parents.get(&object_id).copied().unwrap_or(OBJECT_ROOT)
    }

    fn map_attributes(&self, stat: Stat) -> FuseAttributes {
        let block_size = self.vfs.statfs().block_size;
        FuseAttributes {
            object_id: stat.object_id,
            kind: stat.kind,
            size: stat.size,
            blocks_512: stat.allocated_size.div_ceil(512),
            links: stat.links,
            // The STORED projection, not the mount's configured default.
            // config.file_mode and config.directory_mode are what a new
            // object is created with when its creator names no mode; once an
            // object exists, its own protection word is the answer, or a
            // chmod would report success and change nothing anybody can see.
            mode: stat.mode,
            uid: stat.owner_uid,
            gid: stat.owner_gid,
            created: stat.created,
            modified: stat.modified,
            changed: stat.changed,
            block_size,
        }
    }
}

fn utf8_name(name: &[u8]) -> Result<&str, VfsError> {
    std::str::from_utf8(name).map_err(|_| VfsError::Invalid)
}
