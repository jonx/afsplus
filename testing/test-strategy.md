# Test Strategy

> **ADRs:** none · **Spec:** none ·
> **Tests:** [crash-testing](crash-testing.md) · **Milestones:** M07

## Test pyramid

### Unit

Encoding, checksums, normalization, tree operations, allocation arithmetic, extent merging, checkpoint selection, retirement-generation rules.

### Property

Examples:

- write/read round-trip
- rename preserves object ID
- create then unlink eventually returns capacity after reclamation
- directory iteration returns every reachable entry exactly once
- transaction abort leaves authoritative state unchanged
- no block reachable from a retained checkpoint is allocatable

### Portable-core proof tests

Unlike a handler-only filesystem, `libafsplus` is designed to build natively on development hosts. Test the actual production core directly with deterministic mock block devices and tiny cache budgets.

This serves the same purpose as pfs3aio's current live-source tests without needing to extract individual functions from OS-bound source.

### Image conformance

Known byte-for-byte images with expected output.

### Crash/fault injection

Interrupt and corrupt I/O at deterministic points, including cache eviction and OOM. See `crash-testing.md`.

### Fuzz

Mutate every independent metadata parser and on-disk structure.

### Black-box real handler

Run the actual AROS handler through DOS-facing operations against disposable images/devices. This layer verifies integration that portable-core tests cannot cover:

- locks
- DOS packets/API v2 translation
- notifications
- mount/unmount
- removable media
- application-visible errors

This mirrors the useful separation in current pfs3aio between host-level proof tests and black-box AmiFUSE tests.

### Interoperability

The same image is opened by:

- portable host reader
- FUSE
- AROS handler
- checker

### Application

Git, Cargo, Zed-style watcher, Ferail, Moonstone.

## Reproducibility

Every failure records:

- seed
- image hash
- implementation revision
- active feature set
- operation trace
- cache budget
- fault-injection schedule
- selected checkpoint generation
