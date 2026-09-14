# ADR-094: Transport symlink targets through scoped backup and restore APIs

Status: Accepted
Amends: ADR-068, ADR-075, ADR-077

## Context

Core target storage cannot provide an authorized backup or restore operation by
itself. Reading a symlink as regular-file content is ambiguous and following its
target could escape a captured view or isolated restore destination.

## Decision

Add explicit bounded target reads to the VFS and trusted backup interfaces, and
explicit target creation/readback to destination-scoped restore. Targets are
opaque NUL-free UTF-8 bytes; no operation resolves them. Return the required byte
count without modifying a short output buffer. Optional providers return
NotSupported by default. Hold the original grant through kind validation and
provider access; revocation prevents further operations on existing handles.
Restore creation reserves an active handle before mutation and validates the
parent/name/target under the destination grant. Unsupported or constrained
providers refuse rather than create an ordinary file or truncate the target.

VFS unlink dispatches symlinks directly to metadata retirement without a
regular-file orphan. The Rust VFS advertises its own SYMLINKS capability only
with creation, reads and unlink available. Capability bit identities in the
separate published C API are not renumbered; C and OS adapters require their own
entry points and qualification. Atomic replacement and archive target/profile
binding remain separate operations and are not implied by this capability.

## Qualification

Verify exact bytes and required-size retries, no traversal, read-only and revoked
refusals, unsupported providers, grant lifetime during callbacks, resource
refusal before creation, isolation and remount. Test a live logged-write window
beside namespace operations. An isolated target may contain relative or absolute
host syntax but must not be resolved by any transport function. Preserve separate
native and constrained-runtime qualification.
