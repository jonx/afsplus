use std::fmt::Debug;
use std::io;
use std::io::IoSlice;

/// Message-oriented transport for an already mounted FUSE session.
///
/// A receive call returns exactly one complete FUSE request. A send call
/// transmits exactly one complete FUSE reply assembled from `buffers`.
/// Implementations must preserve these message boundaries and may block.
pub trait SessionTransport: Debug + Send + Sync + 'static {
    /// Receives one complete serialized FUSE request.
    fn receive(&self, buffer: &mut [u8]) -> io::Result<usize>;

    /// Sends one complete serialized FUSE reply.
    fn send(&self, buffers: &[IoSlice<'_>]) -> io::Result<()>;
}
