//! Minimal dynamic bridge to macFUSE's libfuse-2 mount compatibility ABI.
//!
//! Dynamic loading keeps normal builds and protocol tests independent of a
//! machine-wide macFUSE installation. The mounted descriptor speaks the FUSE
//! kernel protocol and is handed to `fuser::Session::from_fd` by the safe host
//! adapter.

use std::ffi::{c_char, c_int, CString};
use std::fmt;
use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use libloading::Library;

const LIBRARY_ENV: &str = "AFSPLUS_MACFUSE_LIBRARY";
const DEFAULT_LIBRARIES: &[&str] = &[
    "/usr/local/lib/libfuse.dylib",
    "/usr/local/lib/libfuse.2.dylib",
    "/Library/Filesystems/macfuse.fs/Contents/Frameworks/libfuse.dylib",
];

#[repr(C)]
struct FuseArgs {
    argc: c_int,
    argv: *const *const c_char,
    allocated: c_int,
}

type FuseMountCompat25 = unsafe extern "C" fn(*const c_char, *const FuseArgs) -> c_int;
type FuseUnmountCompat22 = unsafe extern "C" fn(*const c_char);

/// Keeps the dynamically loaded implementation and mountpoint alive until
/// after the fuser session has closed its descriptor.
pub struct MacFuseMount {
    library: Library,
    mountpoint: CString,
    descriptor: Option<OwnedFd>,
}

impl fmt::Debug for MacFuseMount {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MacFuseMount")
            .field("mountpoint", &self.mountpoint)
            .field("has_descriptor", &self.descriptor.is_some())
            .finish_non_exhaustive()
    }
}

impl MacFuseMount {
    /// Mounts `mountpoint` through macFUSE and owns the protocol descriptor.
    pub fn new(mountpoint: &Path, options: &[String]) -> io::Result<Self> {
        if !cfg!(target_os = "macos") {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "macFUSE is supported only on macOS",
            ));
        }

        let mountpoint = mountpoint.canonicalize()?;
        let mountpoint = CString::new(mountpoint.as_os_str().as_bytes()).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "mountpoint contains a NUL byte",
            )
        })?;
        let library = load_library()?;

        let mut arguments = vec![CString::new("afsplus-mount").expect("static string")];
        for option in options {
            arguments.push(CString::new("-o").expect("static string"));
            arguments.push(CString::new(option.as_str()).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "mount option contains a NUL byte",
                )
            })?);
        }
        let argument_pointers: Vec<_> = arguments.iter().map(|value| value.as_ptr()).collect();
        let fuse_arguments = FuseArgs {
            argc: c_int::try_from(argument_pointers.len()).map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidInput, "too many mount options")
            })?,
            argv: argument_pointers.as_ptr(),
            allocated: 0,
        };

        // SAFETY: macFUSE exports the libfuse-2.6 compatibility signature used
        // here. Every pointer is NUL-terminated and remains valid for the call.
        let descriptor = unsafe {
            let mount = library
                .get::<FuseMountCompat25>(b"fuse_mount_compat25\0")
                .map_err(|error| io::Error::new(io::ErrorKind::Unsupported, error))?;
            mount(mountpoint.as_ptr(), &fuse_arguments)
        };
        if descriptor < 0 {
            return Err(last_os_error("macFUSE rejected the mount"));
        }

        // SAFETY: a successful fuse_mount_compat25 call transfers ownership of
        // one newly opened descriptor to its caller.
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        Ok(MacFuseMount {
            library,
            mountpoint,
            descriptor: Some(descriptor),
        })
    }

    /// Transfers the mounted FUSE descriptor to the request-processing loop.
    pub fn take_descriptor(&mut self) -> io::Result<OwnedFd> {
        self.descriptor.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "macFUSE descriptor was already taken",
            )
        })
    }
}

impl Drop for MacFuseMount {
    fn drop(&mut self) {
        drop(self.descriptor.take());

        // SAFETY: the library is still loaded, the symbol signature is the
        // exported libfuse compatibility ABI, and mountpoint remains valid.
        unsafe {
            if let Ok(unmount) = self
                .library
                .get::<FuseUnmountCompat22>(b"fuse_unmount_compat22\0")
            {
                unmount(self.mountpoint.as_ptr());
            }
        }
    }
}

fn load_library() -> io::Result<Library> {
    let candidates: Vec<PathBuf> = match std::env::var_os(LIBRARY_ENV) {
        Some(path) => vec![PathBuf::from(path)],
        None => DEFAULT_LIBRARIES.iter().map(PathBuf::from).collect(),
    };
    let mut failures = Vec::new();
    for path in candidates {
        // SAFETY: loading macFUSE is the sole purpose of this native boundary;
        // the successful library is retained for the complete mount lifetime.
        match unsafe { Library::new(&path) } {
            Ok(library) => return Ok(library),
            Err(error) => failures.push(format!("{}: {error}", path.display())),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!(
            "cannot load macFUSE (install it or set {LIBRARY_ENV}): {}",
            failures.join("; ")
        ),
    ))
}

fn last_os_error(context: &str) -> io::Error {
    let error = io::Error::last_os_error();
    if error.raw_os_error().is_some_and(|code| code != 0) {
        io::Error::new(error.kind(), format!("{context}: {error}"))
    } else {
        io::Error::other(context)
    }
}
