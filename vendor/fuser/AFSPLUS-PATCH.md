# AFS+ fuser patch

This directory vendors the crates.io source for `fuser` 0.18.0 under its MIT
license. AFS+ carries a narrow transport patch because macFUSE's FSKit backend
is message-oriented and intentionally exposes no `/dev/fuse` descriptor.

The local changes:

- add the public `SessionTransport` interface;
- add `Session::from_transport`;
- route channel receive and send operations through either the existing device
  implementation or a custom message transport;
- reject descriptor-specific passthrough and clone operations on custom
  transports;
- remove descriptor-only trait implementations from transport-backed sessions;
- silence upstream-only dead-code diagnostics caused by building this reduced
  vendored source set.

Device-backed Linux, FreeBSD and macOS paths retain their existing behavior.
The patch should be proposed upstream or removed when fuser provides an
equivalent stable message-transport boundary.
