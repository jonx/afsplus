//! Dynamic bridge to macFUSE's message-oriented `MFMount.framework` API.
//!
//! FSKit deliberately does not expose a `/dev/fuse` descriptor. The channel
//! API instead preserves complete FUSE request and reply boundaries. Dynamic
//! loading keeps ordinary builds independent of a machine-wide macFUSE
//! installation.

use std::ffi::{c_char, c_void, CString};
use std::fmt;
use std::io::{self, IoSlice};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use fuser::SessionTransport;
use libloading::Library;

const LIBRARY_ENV: &str = "AFSPLUS_MFMOUNT_LIBRARY";
const DEFAULT_LIBRARIES: &[&str] = &[
    "/Library/Filesystems/macfuse.fs/Contents/Frameworks/MFMount.framework/Versions/A/MFMount",
    "/Library/Filesystems/macfuse.fs/Contents/Frameworks/MFMount.framework/MFMount",
];

type Reference = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct Iovec {
    base: *mut c_void,
    length: usize,
}

type ChannelCreate = unsafe extern "C" fn() -> Reference;
type ChannelCopyNextMessage = unsafe extern "C" fn(Reference) -> Reference;
type MessageGetBodyBuffers = unsafe extern "C" fn(Reference, *mut *const Iovec) -> isize;
type ChannelSendMessage = unsafe extern "C" fn(Reference, *const Iovec, usize) -> isize;
type ChannelClose = unsafe extern "C" fn(Reference) -> bool;
type Release = unsafe extern "C" fn(Reference);
type Mount = unsafe extern "C" fn(Reference, *const c_char, *const c_char, bool) -> i32;

#[derive(Clone, Copy)]
struct Symbols {
    channel_copy_next_message: ChannelCopyNextMessage,
    message_get_body_buffers: MessageGetBodyBuffers,
    channel_send_message: ChannelSendMessage,
    channel_close: ChannelClose,
    release: Release,
    mount: Mount,
}

struct ChannelInner {
    channel: NonNull<c_void>,
    symbols: Symbols,
    closed: AtomicBool,
    _library: Library,
}

// SAFETY: MFMount documents channel receive interruption and message sending
// as channel operations. The channel is retained for this object's lifetime,
// closure is serialized by `closed`, and all borrowed message buffers are
// consumed before their message reference is released.
unsafe impl Send for ChannelInner {}
// SAFETY: concurrent reply sends are supported by the channel abstraction;
// AFS+ currently runs one receive loop, so receives are not concurrent.
unsafe impl Sync for ChannelInner {}

impl fmt::Debug for ChannelInner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MFChannel")
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl ChannelInner {
    fn close(&self) {
        if !self.closed.swap(true, Ordering::AcqRel) {
            // SAFETY: channel is a live retained MFChannel reference and this
            // transition is performed at most once.
            unsafe {
                (self.symbols.channel_close)(self.channel.as_ptr());
            }
        }
    }

    fn receive_message(&self, destination: &mut [u8]) -> io::Result<usize> {
        if self.closed.load(Ordering::Acquire) {
            return Err(io::Error::from_raw_os_error(libc::ENODEV));
        }

        // SAFETY: channel remains retained by self for the complete call.
        let message = unsafe { (self.symbols.channel_copy_next_message)(self.channel.as_ptr()) };
        let message = NonNull::new(message).ok_or_else(last_os_error)?;
        let guard = MessageGuard {
            message,
            release: self.symbols.release,
        };

        let mut buffers = std::ptr::null();
        // SAFETY: message is live and buffers points to writable pointer storage.
        let count = unsafe {
            (self.symbols.message_get_body_buffers)(guard.message.as_ptr(), &mut buffers)
        };
        if count <= 0 || buffers.is_null() {
            return Err(last_os_error());
        }
        let count = usize::try_from(count).map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "invalid MFMessage iovec count")
        })?;
        // SAFETY: MFMessage owns an array of `count` iovecs for the lifetime of guard.
        let buffers = unsafe { std::slice::from_raw_parts(buffers, count) };
        let total = buffers.iter().try_fold(0usize, |total, buffer| {
            total.checked_add(buffer.length).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "MFMessage body size overflow")
            })
        })?;
        if total > destination.len() {
            return Err(io::Error::from_raw_os_error(libc::EMSGSIZE));
        }

        let mut offset = 0;
        for buffer in buffers {
            if buffer.length != 0 && buffer.base.is_null() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "MFMessage contains a null body buffer",
                ));
            }
            // SAFETY: the borrowed body range is valid while guard is live and
            // destination has been checked to contain the concatenated body.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    buffer.base.cast::<u8>(),
                    destination.as_mut_ptr().add(offset),
                    buffer.length,
                );
            }
            offset += buffer.length;
        }
        if total >= 16 {
            let opcode = u32::from_ne_bytes(destination[4..8].try_into().expect("fixed slice"));
            let unique = u64::from_ne_bytes(destination[8..16].try_into().expect("fixed slice"));
            log::trace!("MFChannel receive: {total} bytes, opcode {opcode}, unique {unique}");
        } else {
            log::trace!("MFChannel receive: {total} bytes");
        }
        Ok(total)
    }

    fn send_message(&self, buffers: &[IoSlice<'_>]) -> io::Result<()> {
        if buffers.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "a FUSE reply must contain at least one buffer",
            ));
        }
        let total = buffers.iter().try_fold(0usize, |total, buffer| {
            total.checked_add(buffer.len()).ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "FUSE reply size overflow")
            })
        })?;
        let native: Vec<_> = buffers
            .iter()
            .map(|buffer| Iovec {
                base: buffer.as_ptr().cast_mut().cast::<c_void>(),
                length: buffer.len(),
            })
            .collect();
        if let Some(header) = reply_header(buffers) {
            let length = u32::from_ne_bytes(header[0..4].try_into().expect("fixed slice"));
            let error = i32::from_ne_bytes(header[4..8].try_into().expect("fixed slice"));
            let unique = u64::from_ne_bytes(header[8..16].try_into().expect("fixed slice"));
            log::trace!(
                "MFChannel send: {total} bytes, header length {length}, error {error}, unique {unique}"
            );
        } else {
            log::trace!("MFChannel send: {total} bytes");
        }
        // SAFETY: channel is retained and every iovec borrows a caller buffer
        // that remains live for the duration of this synchronous call.
        let sent = unsafe {
            (self.symbols.channel_send_message)(
                self.channel.as_ptr(),
                native.as_ptr(),
                native.len(),
            )
        };
        if sent < 0 {
            return Err(last_os_error());
        }
        if usize::try_from(sent).ok() != Some(total) {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                format!("MFChannel sent {sent} of {total} reply bytes"),
            ));
        }
        Ok(())
    }
}

impl SessionTransport for ChannelInner {
    fn receive(&self, buffer: &mut [u8]) -> io::Result<usize> {
        self.receive_message(buffer)
    }

    fn send(&self, buffers: &[IoSlice<'_>]) -> io::Result<()> {
        self.send_message(buffers)
    }
}

impl Drop for ChannelInner {
    fn drop(&mut self) {
        self.close();
        // SAFETY: the Create-owned reference is released exactly once, after
        // the channel has closed and before the library field is dropped.
        unsafe {
            (self.symbols.release)(self.channel.as_ptr());
        }
    }
}

struct MessageGuard {
    message: NonNull<c_void>,
    release: Release,
}

impl Drop for MessageGuard {
    fn drop(&mut self) {
        // SAFETY: this guard owns the Copy-created message reference.
        unsafe {
            (self.release)(self.message.as_ptr());
        }
    }
}

/// Owns one MFMount channel and the asynchronous mount operation.
pub struct MacFuseMount {
    inner: Arc<ChannelInner>,
    mount_thread: Option<JoinHandle<io::Result<()>>>,
    mount_complete: bool,
}

impl fmt::Debug for MacFuseMount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacFuseMount")
            .field("channel", &self.inner)
            .field("mount_complete", &self.mount_complete)
            .finish_non_exhaustive()
    }
}

impl MacFuseMount {
    /// Begins an FSKit mount while leaving the channel available for the FUSE
    /// INIT handshake. Call [`Self::wait_until_mounted`] after constructing the
    /// fuser session.
    pub fn new(mountpoint: &Path, options: &[String]) -> io::Result<Self> {
        if !cfg!(target_os = "macos") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "macFUSE is supported only on macOS",
            ));
        }

        let mountpoint = resolve_mountpoint(mountpoint)?;
        let mountpoint = CString::new(mountpoint.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "mountpoint contains a NUL byte",
            )
        })?;
        let options = CString::new(options.join(",")).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "mount options contain a NUL byte",
            )
        })?;

        let library = load_library()?;
        // SAFETY: every symbol is resolved from the retained MFMount framework
        // using the signature published by its installed C header.
        let (create, symbols) = unsafe { load_symbols(&library)? };
        // SAFETY: no arguments are required and the framework is retained.
        let channel = NonNull::new(unsafe { create() })
            .ok_or_else(|| io::Error::other("MFChannelCreate failed"))?;
        let inner = Arc::new(ChannelInner {
            channel,
            symbols,
            closed: AtomicBool::new(false),
            _library: library,
        });
        let mount_inner = inner.clone();
        let mount_thread = thread::Builder::new()
            .name("afsplus-mfmount".into())
            .spawn(move || {
                // SAFETY: the channel and C strings remain valid for the call.
                let result = unsafe {
                    (mount_inner.symbols.mount)(
                        mount_inner.channel.as_ptr(),
                        mountpoint.as_ptr(),
                        options.as_ptr(),
                        false,
                    )
                };
                let outcome = mount_result(result);
                if outcome.is_err() {
                    mount_inner.close();
                }
                outcome
            })?;

        Ok(MacFuseMount {
            inner,
            mount_thread: Some(mount_thread),
            mount_complete: false,
        })
    }

    /// Returns the transport used to construct `fuser::Session`.
    pub fn transport(&self) -> Arc<dyn SessionTransport> {
        self.inner.clone()
    }

    /// Waits for macFUSE to finish the mount operation after the FUSE INIT
    /// handshake has completed.
    pub fn wait_until_mounted(&mut self) -> io::Result<()> {
        if self.mount_complete {
            return Ok(());
        }
        let thread = self
            .mount_thread
            .take()
            .ok_or_else(|| io::Error::other("MFMount thread is missing"))?;
        let result = thread
            .join()
            .map_err(|_| io::Error::other("MFMount thread panicked"))?;
        result?;
        self.mount_complete = true;
        Ok(())
    }
}

impl Drop for MacFuseMount {
    fn drop(&mut self) {
        self.inner.close();
        if let Some(thread) = self.mount_thread.take() {
            let _ = thread.join();
        }
    }
}

unsafe fn load_symbols(library: &Library) -> io::Result<(ChannelCreate, Symbols)> {
    // SAFETY: callers retain library for every copied function pointer.
    unsafe {
        Ok((
            load_symbol(library, b"MFChannelCreate\0")?,
            Symbols {
                channel_copy_next_message: load_symbol(library, b"MFChannelCopyNextMessage\0")?,
                message_get_body_buffers: load_symbol(library, b"MFMessageGetBodyBuffers\0")?,
                channel_send_message: load_symbol(library, b"MFChannelSendMessage\0")?,
                channel_close: load_symbol(library, b"MFChannelClose\0")?,
                release: load_symbol(library, b"MFRelease\0")?,
                mount: load_symbol(library, b"MFMount\0")?,
            },
        ))
    }
}

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> io::Result<T> {
    // SAFETY: the caller supplies the exact ABI type for a named framework symbol.
    unsafe {
        library
            .get::<T>(name)
            .map(|symbol| *symbol)
            .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))
    }
}

fn load_library() -> io::Result<Library> {
    let candidates: Vec<PathBuf> = match std::env::var_os(LIBRARY_ENV) {
        Some(path) => vec![PathBuf::from(path)],
        None => DEFAULT_LIBRARIES.iter().map(PathBuf::from).collect(),
    };
    let mut failures = Vec::new();
    for path in candidates {
        // SAFETY: loading MFMount is the purpose of this audited boundary and
        // a successful library remains retained by ChannelInner.
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok(library),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "cannot load MFMount.framework (install macFUSE or set {LIBRARY_ENV}): {}",
            failures.join("; ")
        ),
    ))
}

fn resolve_mountpoint(mountpoint: &Path) -> io::Result<PathBuf> {
    match mountpoint.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = mountpoint.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "mountpoint has no parent")
            })?;
            let name = mountpoint.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "mountpoint has no final component",
                )
            })?;
            Ok(parent.canonicalize()?.join(name))
        }
        Err(error) => Err(error),
    }
}

fn mount_result(result: i32) -> io::Result<()> {
    match result {
        0 => Ok(()),
        1 => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "MFMount does not support this macOS version",
        )),
        2 => Err(io::Error::other(
            "macFUSE helper tools could not be installed",
        )),
        3 => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "macFUSE file-system extension was not found",
        )),
        4 => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "macFUSE file-system extension is not enabled",
        )),
        -1 => Err(last_os_error()),
        value => Err(io::Error::other(format!(
            "MFMount returned unknown result {value}"
        ))),
    }
}

fn last_os_error() -> io::Error {
    let error = io::Error::last_os_error();
    if error.raw_os_error().is_some_and(|code| code != 0) {
        error
    } else {
        io::Error::other("MFMount operation failed without errno")
    }
}

fn reply_header(buffers: &[IoSlice<'_>]) -> Option<[u8; 16]> {
    let mut header = [0; 16];
    let mut copied = 0;
    for buffer in buffers {
        let count = (header.len() - copied).min(buffer.len());
        header[copied..copied + count].copy_from_slice(&buffer[..count]);
        copied += count;
        if copied == header.len() {
            return Some(header);
        }
    }
    None
}
