use std::io;
use std::os::fd::BorrowedFd;
use std::sync::Arc;

use nix::errno::Errno;

use crate::dev_fuse::DevFuse;
use crate::passthrough::BackingId;
use crate::SessionTransport;

#[derive(Debug, Clone)]
enum ChannelInner {
    Device(Arc<DevFuse>),
    Transport(Arc<dyn SessionTransport>),
}

/// A raw communication channel to a FUSE backend.
#[derive(Debug, Clone)]
pub(crate) struct Channel(ChannelInner);

impl Channel {
    /// Create a new communication channel to the kernel driver by mounting the
    /// given path. The kernel driver will delegate filesystem operations of
    /// the given path to the channel.
    pub(crate) fn new(device: Arc<DevFuse>) -> Self {
        Self(ChannelInner::Device(device))
    }

    pub(crate) fn from_transport(transport: Arc<dyn SessionTransport>) -> Self {
        Self(ChannelInner::Transport(transport))
    }

    /// Receives data up to the capacity of the given buffer (can block).
    fn receive(&self, buffer: &mut [u8]) -> nix::Result<usize> {
        match &self.0 {
            ChannelInner::Device(device) => nix::unistd::read(device, buffer),
            ChannelInner::Transport(transport) => transport
                .receive(buffer)
                .map_err(|error| Errno::from_raw(error.raw_os_error().unwrap_or(libc::EIO))),
        }
    }

    /// Receives data up to the capacity of the given buffer (can block),
    /// retrying on errors that are safe to retry (ENOENT, EINTR, EAGAIN).
    ///
    /// - ENOENT: Operation interrupted. According to FUSE, this is safe to retry.
    /// - EINTR: Interrupted system call, retry.
    /// - EAGAIN: Explicitly instructed to try again.
    pub(crate) fn receive_retrying(&self, buffer: &mut [u8]) -> nix::Result<usize> {
        loop {
            match self.receive(buffer) {
                Ok(size) => return Ok(size),
                Err(Errno::ENOENT | Errno::EINTR | Errno::EAGAIN) => continue,
                Err(err) => return Err(err),
            }
        }
    }

    /// Returns a sender object for this channel. The sender object can be
    /// used to send to the channel. Multiple sender objects can be used
    /// and they can safely be sent to other threads.
    pub(crate) fn sender(&self) -> ChannelSender {
        // Since write/writev syscalls are threadsafe, we can simply create
        // a sender by using the same file and use it in other threads.
        ChannelSender(self.0.clone())
    }

    /// Clone the FUSE device fd using FUSE_DEV_IOC_CLONE ioctl.
    ///
    /// This creates a new fd that can read FUSE requests independently,
    /// enabling true parallel request processing. The kernel distributes
    /// requests across all cloned fds.
    ///
    /// Requires Linux 4.5+. Returns an error on older kernels or non-Linux.
    #[cfg(target_os = "linux")]
    pub(crate) fn clone_fd(&self) -> io::Result<Channel> {
        use std::os::fd::AsRawFd;

        let ChannelInner::Device(device) = &self.0 else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "custom FUSE transports cannot clone a device descriptor",
            ));
        };
        let new_dev = DevFuse::open()?;
        let mut source_fd = device.as_raw_fd() as u32;
        // SAFETY: fuse_dev_ioc_clone is a valid ioctl for /dev/fuse
        unsafe {
            crate::ll::ioctl::fuse_dev_ioc_clone(new_dev.as_raw_fd(), &mut source_fd)?;
        }

        Ok(Channel::new(Arc::new(new_dev)))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ChannelSender(ChannelInner);

impl ChannelSender {
    pub(crate) fn send(&self, bufs: &[io::IoSlice<'_>]) -> io::Result<()> {
        match &self.0 {
            ChannelInner::Device(device) => {
                let rc = nix::sys::uio::writev(device, bufs)?;
                // writev is atomic, so do not need to check how many bytes are written.
                // libfuse does not do it either
                // https://github.com/libfuse/libfuse/blob/6278995cca991978abd25ebb2c20ebd3fc9e8a13/lib/fuse_lowlevel.c#L267
                debug_assert_eq!(bufs.iter().map(|b| b.len()).sum::<usize>(), rc);
                Ok(())
            }
            ChannelInner::Transport(transport) => transport.send(bufs),
        }
    }

    pub(crate) fn open_backing(&self, fd: BorrowedFd<'_>) -> std::io::Result<BackingId> {
        match &self.0 {
            ChannelInner::Device(device) => BackingId::create(device, fd),
            ChannelInner::Transport(_) => Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "custom FUSE transports do not support passthrough backing files",
            )),
        }
    }

    pub(crate) unsafe fn wrap_backing(&self, id: u32) -> BackingId {
        match &self.0 {
            ChannelInner::Device(device) => unsafe { BackingId::wrap_raw(device, id) },
            ChannelInner::Transport(_) => {
                panic!("custom FUSE transports do not support passthrough backing files")
            }
        }
    }
}
