# Implementation Plan

Progress notation: plain `M01` means not started, `[M01]` means started or
partial, and ~~M01~~ means complete. Stage labels follow the same convention.
Links retain the visible brackets or strikethrough. `make toc` refreshes these
labels from the milestone status cells; `make check-docs` detects stale labels.
Stage completion requires every contributing milestone to be complete; Stage 0
tracks ongoing design review separately and is excluded from finite completion. Prototype completion
with open qualification is partial. Completed individual list entries are also
struck through, using the [item records](milestones.md#individual-list-item-completion).
A completed component does not complete its phase's separate acceptance gates.
Phase 0 entries require a frozen contract, rather than an executable codec alone.


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

> **Roadmap:** [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M00](milestones.md)

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

> **Roadmap:** [~~Stage A~~](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage D\]](../ROADMAP.md#stage-d-portability-and-host-tooling) · **Milestones:** [\[M01\]](milestones.md), [\[M12\]](milestones.md)

Implement:

- ~~host-file block backend~~ <!-- progress: implementation-01 -->
- ~~superblock discovery~~ <!-- progress: implementation-02 -->
- ~~feature negotiation~~ <!-- progress: implementation-03 -->
- ~~object read~~ <!-- progress: implementation-04 -->
- ~~directory lookup/iteration~~ <!-- progress: implementation-05 -->
- ~~extent read~~ <!-- progress: implementation-06 -->
- ~~metadata validation~~ <!-- progress: implementation-07 -->

Acceptance:

- reads all conformance images
- bounded-memory tests
- ~~fuzz targets active~~ <!-- progress: implementation-18 -->
- builds on macOS/Linux and at least one AROS target

## Phase 2: formatter and image builder

> **Roadmap:** [~~Stage A~~](../ROADMAP.md#stage-a-make-the-core-executable) · **Milestones:** [~~M02~~](milestones.md)

Implement `mkafsplus`.

Acceptance:

- ~~deterministic test mode~~ <!-- progress: implementation-08 -->
- ~~round-trip reader tests~~ <!-- progress: implementation-09 -->
- ~~no native struct serialization~~ <!-- progress: implementation-10 -->

## Phase 3: allocator and mutations

> **Roadmap:** [~~Stage A~~](../ROADMAP.md#stage-a-make-the-core-executable), [~~Stage B~~](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) · **Milestones:** [\[M03\]](milestones.md)

Implement:

- ~~allocation regions~~ <!-- progress: implementation-11 -->
- ~~file create/write/truncate~~ <!-- progress: implementation-12 -->
- ~~mkdir~~ <!-- progress: implementation-13 -->
- ~~unlink~~ <!-- progress: implementation-14 -->
- ~~rename~~ <!-- progress: implementation-15 -->
- ~~hard links~~ <!-- progress: implementation-16 -->
- symlinks <!-- progress: implementation-17 -->

Initially run only against disposable images.

## Phase 4: journal

> **Roadmap:** [~~Stage A~~](../ROADMAP.md#stage-a-make-the-core-executable), [~~Stage B~~](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers) · **Milestones:** [\[M04\]](milestones.md)

Implement the checkpoint-COW/group-commit/intent-log contract of
[ADR-063](../adr/ADR-063-intent-log-epoch1.md), with existing-file write/truncate
records under [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md).

Acceptance:

- power-failure injection after every metadata write point
- recovered image always satisfies core invariants

## Phase 5: checker

> **Roadmap:** [~~Stage A~~](../ROADMAP.md#stage-a-make-the-core-executable), [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [\[M05\]](milestones.md)

Implement `afsplus-check`.

It shares format and invariant code with `libafsplus`.

## Phase 6: AROS handler

> **Roadmap:** [\[Stage C\]](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability) · **Milestones:** [\[M06\]](milestones.md)

Map DOS operations onto the portable core.

Preserve classic application ABI.

## Phase 7: Filesystem API v2

> **Roadmap:** [\[Stage C\]](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability) · **Milestones:** [\[M07\]](milestones.md)

Implement capability discovery and 64-bit modern operations.

Build legacy adapter.

## Phase 8: FUSE

> **Roadmap:** [\[Stage D\]](../ROADMAP.md#stage-d-portability-and-host-tooling) · **Milestones:** [~~M08~~](milestones.md)

Mount images on macOS/Linux.

This is a release gate for format portability.

## Phase 9: global catalog

> **Roadmap:** [Stage E](../ROADMAP.md#stage-e-developer-contract-accelerators-and-optional-features) · **Milestones:** [M09](milestones.md)

Implement catalog build, validation, streaming enumeration, invalidation, rebuild.

## Phase 10: change stream

> **Roadmap:** [Stage E](../ROADMAP.md#stage-e-developer-contract-accelerators-and-optional-features) · **Milestones:** [M10](milestones.md)

Implement persistent sequence log and `RESCAN_REQUIRED` fallback.

## Phase 11: resize and maintenance

> **Roadmap:** [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M11](milestones.md)

Implement grow first.

Shrink follows only after safe relocation and minimum-size analysis are proven.

## Phase 12: application qualification

> **Roadmap:** [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [\[M13\]](milestones.md)

Qualify:

- Rust/Cargo
- Git
- Zed-style worktree
- Ferail
- Moonstone
- large media files
- low-memory profile

## Snapshot and backup integration across phases

> **Roadmap:** [~~Stage B~~](../ROADMAP.md#stage-b-resolve-the-epoch-1-architecture-blockers), [\[Stage C\]](../ROADMAP.md#stage-c-integrate-aros-and-begin-independent-c-portability), [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [\[M03\]](milestones.md), [\[M04\]](milestones.md), [\[M07\]](milestones.md), [\[M13\]](milestones.md), [\[M14\]](milestones.md)

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

> **Roadmap:** [\[Stage F\]](../ROADMAP.md#stage-f-production-qualification) · **Milestones:** [M00](milestones.md), [\[M14\]](milestones.md)

Only after:

- conformance suite
- crash tests
- ~~FUSE interop~~ <!-- progress: implementation-19 -->
- AROS real-disk testing
- independent reader review
- specification audit
