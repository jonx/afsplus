# Implementation Plan

The implementation is deliberately staged so the on-disk format is exercised on host files before any AROS disk is at risk.

## Phase 0: specification freeze for reader subset

Deliver:

- terminology
- identification block
- superblock
- endian helpers
- checksum
- object record
- directory leaf format
- extent format
- allocation-region format
- feature-record format

No read/write handler work begins until a reader can parse reference images.

## Phase 1: portable reader

Implement:

- host-file block backend
- superblock discovery
- feature negotiation
- object read
- directory lookup/iteration
- extent read
- metadata validation

Acceptance:

- reads all conformance images
- bounded-memory tests
- fuzz targets active
- builds on macOS/Linux and at least one AROS target

## Phase 2: formatter and image builder

Implement `mkafsplus`.

Acceptance:

- deterministic test mode
- round-trip reader tests
- no native struct serialization

## Phase 3: allocator and mutations

Implement:

- allocation regions
- file create/write/truncate
- mkdir
- unlink
- rename
- hard links
- symlinks

Initially run only against disposable images.

## Phase 4: journal

Implement transaction log and crash recovery.

Acceptance:

- power-failure injection after every metadata write point
- recovered image always satisfies core invariants

## Phase 5: checker

Implement `afsplus-check`.

It shares format and invariant code with `libafsplus`.

## Phase 6: AROS handler

Map DOS operations onto the portable core.

Preserve classic application ABI.

## Phase 7: Filesystem API v2

Implement capability discovery and 64-bit modern operations.

Build legacy adapter.

## Phase 8: FUSE

Mount images on macOS/Linux.

This is a release gate for format portability.

## Phase 9: global catalog

Implement catalog build, validation, streaming enumeration, invalidation, rebuild.

## Phase 10: change stream

Implement persistent sequence log and `RESCAN_REQUIRED` fallback.

## Phase 11: resize and maintenance

Implement grow first.

Shrink follows only after safe relocation and minimum-size analysis are proven.

## Phase 12: application qualification

Qualify:

- Rust/Cargo
- Git
- Zed-style worktree
- Ferail
- Moonstone
- large media files
- low-memory profile

## Phase 13: epoch 1 freeze

Only after:

- conformance suite
- crash tests
- FUSE interop
- AROS real-disk testing
- independent reader review
- specification audit
