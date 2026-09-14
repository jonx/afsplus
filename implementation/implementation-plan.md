# Implementation Plan

The implementation is deliberately staged so the on-disk format is exercised on host files before any AROS disk is at risk.

Stages in the [roadmap](../ROADMAP.md) group related work; phases below name
implementation deliverables. Phase numbers and milestone numbers are separate.
A milestone can contribute to several stages. Completion and remaining
qualification are recorded in [milestones](milestones.md); immediate work is
in the [audit queue](audit-work-queue.md).

<!-- toc -->

- [Phase 0: specification freeze for reader subset](#phase-0-specification-freeze-for-reader-subset)
- [Phase 1: portable reader](#phase-1-portable-reader)
- [Phase 2: formatter and image builder](#phase-2-formatter-and-image-builder)
- [Phase 3: allocator and mutations](#phase-3-allocator-and-mutations)
- [Phase 4: journal](#phase-4-journal)
- [Phase 5: checker](#phase-5-checker)
- [Phase 6: AROS handler](#phase-6-aros-handler)
- [Phase 7: Filesystem API v2](#phase-7-filesystem-api-v2)
- [Phase 8: FUSE](#phase-8-fuse)
- [Phase 9: global catalog](#phase-9-global-catalog)
- [Phase 10: change stream](#phase-10-change-stream)
- [Phase 11: resize and maintenance](#phase-11-resize-and-maintenance)
- [Phase 12: application qualification](#phase-12-application-qualification)
- [Snapshot and backup integration across phases](#snapshot-and-backup-integration-across-phases)
- [Phase 13: epoch 1 freeze](#phase-13-epoch-1-freeze)

<!-- /toc -->

## Phase 0: specification freeze for reader subset

> **Roadmap:** [Stage A](../ROADMAP.md#stage-a-make-the-core-executable), [Stage B](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers), [Stage F](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M00](milestones.md)

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

> **Roadmap:** [Stage A](../ROADMAP.md#stage-a-make-the-core-executable), [Stage D](../ROADMAP.md#stage-d-portability-and-host-tooling) · **Milestones:** [M01](milestones.md), [M12](milestones.md)

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

> **Roadmap:** [Stage A](../ROADMAP.md#stage-a-make-the-core-executable) · **Milestones:** [M02](milestones.md)

Implement `mkafsplus`.

Acceptance:

- deterministic test mode
- round-trip reader tests
- no native struct serialization

## Phase 3: allocator and mutations

> **Roadmap:** [Stage A](../ROADMAP.md#stage-a-make-the-core-executable), [Stage B](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) · **Milestones:** [M03](milestones.md)

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

> **Roadmap:** [Stage A](../ROADMAP.md#stage-a-make-the-core-executable), [Stage B](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) · **Milestones:** [M04](milestones.md)

Implement the checkpoint-COW/group-commit/intent-log contract of
[ADR-063](../adr/ADR-063-intent-log-epoch1.md), with existing-file write/truncate
records under [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md).

Acceptance:

- power-failure injection after every metadata write point
- recovered image always satisfies core invariants

## Phase 5: checker

> **Roadmap:** [Stage A](../ROADMAP.md#stage-a-make-the-core-executable), [Stage F](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M05](milestones.md)

Implement `afsplus-check`.

It shares format and invariant code with `libafsplus`.

## Phase 6: AROS handler

> **Roadmap:** [Stage C](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability) · **Milestones:** [M06](milestones.md)

Map DOS operations onto the portable core.

Preserve classic application ABI.

## Phase 7: Filesystem API v2

> **Roadmap:** [Stage C](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability) · **Milestones:** [M07](milestones.md)

Implement capability discovery and 64-bit modern operations.

Build legacy adapter.

## Phase 8: FUSE

> **Roadmap:** [Stage D](../ROADMAP.md#stage-d-portability-and-host-tooling) · **Milestones:** [M08](milestones.md)

Mount images on macOS/Linux.

This is a release gate for format portability.

## Phase 9: global catalog

> **Roadmap:** [Stage E](../ROADMAP.md#stage-e-developer-contract-accelerators-and-optional-features) · **Milestones:** [M09](milestones.md)

Implement catalog build, validation, streaming enumeration, invalidation, rebuild.

## Phase 10: change stream

> **Roadmap:** [Stage E](../ROADMAP.md#stage-e-developer-contract-accelerators-and-optional-features) · **Milestones:** [M10](milestones.md)

Implement persistent sequence log and `RESCAN_REQUIRED` fallback.

## Phase 11: resize and maintenance

> **Roadmap:** [Stage F](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M11](milestones.md)

Implement grow first.

Shrink follows only after safe relocation and minimum-size analysis are proven.

## Phase 12: application qualification

> **Roadmap:** [Stage F](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M13](milestones.md)

Qualify:

- Rust/Cargo
- Git
- Zed-style worktree
- Ferail
- Moonstone
- large media files
- low-memory profile

## Snapshot and backup integration across phases

> **Roadmap:** [Stage B](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers), [Stage C](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability), [Stage F](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M03, M04, M07, M13, M14](milestones.md)

This work connects core lifetime management, the capability API and application
qualification without renumbering phases.

- Integrate persistent snapshots, lifetime accounting, bounded maintenance and
  exact retained-view reads through mutation, release, remount and failures.
- Bind revocable backup and destination-scoped restore authority to actual host
  providers; preserve explicit resource limits and metadata inventory knowledge.
- Combine file, directory, hard-link and symlink archive components into a
  complete job with verified contents, metadata, allocation and finalization.
- Qualify whole-job preservation, explicit recovery losses, cancellation,
  constrained memory, crash boundaries and target adapters.

The [audit queue](audit-work-queue.md) owns detailed integration order;
[open questions](open-questions.md) owns retention/admission policy, rich security
semantics and existing-destination restoration decisions. Component tests do
not establish whole-job or hardware acceptance.

## Phase 13: epoch 1 freeze

> **Roadmap:** [Stage F](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M00](milestones.md), [M14](milestones.md)

Only after:

- conformance suite
- crash tests
- FUSE interop
- AROS real-disk testing
- independent reader review
- specification audit
