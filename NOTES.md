# Notes

The project journal: what was decided, tried and delivered, newest first.
This is the only document that narrates. Everything else states the finished
state and links here for the story; see
[docs/DOCUMENTATION.md](docs/DOCUMENTATION.md) for the rules.

Entry format: `## YYYY-MM-DD — title`.

<!-- toc -->

- [2026-09-14 - Audit finite executable-core stage requirements](#2026-09-14---audit-finite-executable-core-stage-requirements)
- [2026-09-14 - Preserve scoped symlinks in bound archive groups](#2026-09-14---preserve-scoped-symlinks-in-bound-archive-groups)
- [2026-09-14 - Separate ongoing review from finite stage completion](#2026-09-14---separate-ongoing-review-from-finite-stage-completion)
- [2026-09-14 — Generate visible milestone and stage progress](#2026-09-14--generate-visible-milestone-and-stage-progress)
- [2026-09-14 - Add scoped symlink transport and VFS dispatch](#2026-09-14---add-scoped-symlink-transport-and-vfs-dispatch)
- [2026-09-14 — Refresh implementation navigation and remaining work](#2026-09-14--refresh-implementation-navigation-and-remaining-work)
- [2026-09-14 - Preserve symlink targets through core namespace transactions](#2026-09-14---preserve-symlink-targets-through-core-namespace-transactions)
- [2026-09-14 — Simplify ADR decision statuses](#2026-09-14--simplify-adr-decision-statuses)
- [2026-09-14 - Cross-read inline symlink records with Rust and C](#2026-09-14---cross-read-inline-symlink-records-with-rust-and-c)
- [2026-09-14 - Reject undersized object encoder buffers](#2026-09-14---reject-undersized-object-encoder-buffers)
- [2026-09-14 - Bind directory and alias archive restoration](#2026-09-14---bind-directory-and-alias-archive-restoration)
- [2026-09-14 - Reconcile the filesystem comparison with executable support](#2026-09-14---reconcile-the-filesystem-comparison-with-executable-support)
- [2026-09-14 - Reopen created restore entries with bounded active handles](#2026-09-14---reopen-created-restore-entries-with-bounded-active-handles)
- [2026-09-14 - Bind exact regular-file metadata to allocation and opaque inventories](#2026-09-14---bind-exact-regular-file-metadata-to-allocation-and-opaque-inventories)
- [2026-09-14 - Bind archive allocation records to verified sparse restoration](#2026-09-14---bind-archive-allocation-records-to-verified-sparse-restoration)
- [2026-09-14 - Add scoped allocation readback for full restore verification](#2026-09-14---add-scoped-allocation-readback-for-full-restore-verification)
- [2026-09-14 - Transport captured sparse contents through the archive consumer](#2026-09-14---transport-captured-sparse-contents-through-the-archive-consumer)
- [2026-09-14 - Exercise repeated low-space reclamation with retained snapshots](#2026-09-14---exercise-repeated-low-space-reclamation-with-retained-snapshots)
- [2026-09-14 - Bind full object inventories to counted descriptor manifests](#2026-09-14---bind-full-object-inventories-to-counted-descriptor-manifests)
- [2026-09-14 - Replay verified scratch archives with bounded upload slots](#2026-09-14---replay-verified-scratch-archives-with-bounded-upload-slots)
- [2026-09-14 - Carry opaque values through an authorized archive consumer](#2026-09-14---carry-opaque-values-through-an-authorized-archive-consumer)
- [2026-09-14 - Stage opaque destination metadata before atomic publication](#2026-09-14---stage-opaque-destination-metadata-before-atomic-publication)
- [2026-09-14 - Stream opaque captured attributes and security metadata](#2026-09-14---stream-opaque-captured-attributes-and-security-metadata)
- [2026-09-14 - Inspect captured metadata knowledge through backup authority](#2026-09-14---inspect-captured-metadata-knowledge-through-backup-authority)
- [2026-09-14 - Preserve exact object metadata and inventory knowledge](#2026-09-14---preserve-exact-object-metadata-and-inventory-knowledge)
- [2026-09-14 - Bind local PAX records to streamed ordinary members](#2026-09-14---bind-local-pax-records-to-streamed-ordinary-members)
- [2026-09-14 - Validate effective PAX member fields before restoration](#2026-09-14---validate-effective-pax-member-fields-before-restoration)
- [2026-09-14 - Bind archive integrity and termination to an independently checked envelope](#2026-09-14---bind-archive-integrity-and-termination-to-an-independently-checked-envelope)
- [2026-09-14 - Add streamed tar framing and independent recovery checks](#2026-09-14---add-streamed-tar-framing-and-independent-recovery-checks)
- [2026-09-14 - Add bounded PAX record parsing for the archive consumer](#2026-09-14---add-bounded-pax-record-parsing-for-the-archive-consumer)
- [2026-09-14 - Bound shrink working sets and retirements](#2026-09-14---bound-shrink-working-sets-and-retirements)
- [2026-09-14 - Grow sparse files without enumerating mappings](#2026-09-14---grow-sparse-files-without-enumerating-mappings)
- [2026-09-14 - Bound atomic writes for restore consumers](#2026-09-14---bound-atomic-writes-for-restore-consumers)
- [2026-09-14 - Bound reservation edits in fragmented files](#2026-09-14---bound-reservation-edits-in-fragmented-files)
- [2026-09-14 — Cross-read initialized reservations through the portable C reader](#2026-09-14--cross-read-initialized-reservations-through-the-portable-c-reader)
- [2026-09-14 — Initialize private unwritten reservations without replacement data allocation](#2026-09-14--initialize-private-unwritten-reservations-without-replacement-data-allocation)
- [2026-09-14 — Add checked destination reservation restoration](#2026-09-14--add-checked-destination-reservation-restoration)
- [2026-09-14 — Delegate decisions while the owner is away](#2026-09-14--delegate-decisions-while-the-owner-is-away)
- [2026-09-14 — Enumerate captured allocation and decide preservation modes](#2026-09-14--enumerate-captured-allocation-and-decide-preservation-modes)
- [2026-09-14 — Enforce separate destination restore grants](#2026-09-14--enforce-separate-destination-restore-grants)
- [2026-09-14 — Restore existing metadata through the common COW tail](#2026-09-14--restore-existing-metadata-through-the-common-cow-tail)
- [2026-09-14 — Enforce revocable authority on the backup interface](#2026-09-14--enforce-revocable-authority-on-the-backup-interface)
- [2026-09-14 — Configure snapshot limits before writable mount recovery](#2026-09-14--configure-snapshot-limits-before-writable-mount-recovery)
- [2026-09-14 — Protect both checkpoint generations during reclamation](#2026-09-14--protect-both-checkpoint-generations-during-reclamation)
- [2026-09-14 — Verify snapshot ownership and select stronger recovery retention](#2026-09-14--verify-snapshot-ownership-and-select-stronger-recovery-retention)
- [2026-09-14 — Integrate persistent snapshot lifecycle into Volume](#2026-09-14--integrate-persistent-snapshot-lifecycle-into-volume)
- [2026-09-14 - Bind lifetime accounting to allocator transactions](#2026-09-14---bind-lifetime-accounting-to-allocator-transactions)
- [2026-09-14 - Prepare transactional lifetime edits](#2026-09-14---prepare-transactional-lifetime-edits)
- [2026-09-14 - Read snapshot trees with bounded contextual checks](#2026-09-14---read-snapshot-trees-with-bounded-contextual-checks)
- [2026-09-14 - Bind snapshot roots in the checkpoint codec](#2026-09-14---bind-snapshot-roots-in-the-checkpoint-codec)
- [2026-09-14 - Add bounded key pages for persistent maintenance cursors](#2026-09-14---add-bounded-key-pages-for-persistent-maintenance-cursors)
- [2026-09-13 - Add persistent snapshot record codecs](#2026-09-13---add-persistent-snapshot-record-codecs)
- [2026-09-13 - Accept the integrated lifetime experiment and handle rule](#2026-09-13---accept-the-integrated-lifetime-experiment-and-handle-rule)
- [2026-09-13 - Add checkpoint extraction with explicit loss reporting](#2026-09-13---add-checkpoint-extraction-with-explicit-loss-reporting)
- [2026-09-13 - Prepare the integrated snapshot accounting decision](#2026-09-13---prepare-the-integrated-snapshot-accounting-decision)
- [2026-09-13 - Measure snapshot retention and reclaim traversal](#2026-09-13---measure-snapshot-retention-and-reclaim-traversal)
- [2026-09-13 — Prepare the complete implementation handoff](#2026-09-13--prepare-the-complete-implementation-handoff)
- [2026-09-13 — Exercise repeated pressure and define the snapshot experiment](#2026-09-13--exercise-repeated-pressure-and-define-the-snapshot-experiment)
- [2026-09-13 — Reconcile invariants and block writes after uncertain publication](#2026-09-13--reconcile-invariants-and-block-writes-after-uncertain-publication)
- [2026-09-13 — Complete BFS book review and fragmented allocation regression](#2026-09-13--complete-bfs-book-review-and-fragmented-allocation-regression)
- [2026-09-03 — Portable C allocates and logs its first COW data block](#2026-09-03--portable-c-allocates-and-logs-its-first-cow-data-block)
- [2026-09-03 — Portable C emits its first version-3 data mutation](#2026-09-03--portable-c-emits-its-first-version-3-data-mutation)
- [2026-09-03 — Portable C creates empty files without an allocator](#2026-09-03--portable-c-creates-empty-files-without-an-allocator)
- [2026-09-03 — Portable C trades caller RAM for 7x fewer writer reads](#2026-09-03--portable-c-trades-caller-ram-for-7x-fewer-writer-reads)
- [2026-09-03 — Intent replay makes final namespace removal bounded](#2026-09-03--intent-replay-makes-final-namespace-removal-bounded)
- [2026-09-03 — Portable C performs its first durable mutation](#2026-09-03--portable-c-performs-its-first-durable-mutation)
- [2026-09-03 — Q3 allocation architecture accepted](#2026-09-03--q3-allocation-architecture-accepted)
- [2026-09-03 — Emergency headroom makes ENOSPC recoverable](#2026-09-03--emergency-headroom-makes-enospc-recoverable)
- [2026-09-03 — Open-unlinked files gain a bounded crash-restartable lifetime](#2026-09-03--open-unlinked-files-gain-a-bounded-crash-restartable-lifetime)
- [2026-09-03 — Q3 qualification exposes the missing emergency reserve](#2026-09-03--q3-qualification-exposes-the-missing-emergency-reserve)
- [2026-09-03 — Checkpoint selection regains Rust/C parity](#2026-09-03--checkpoint-selection-regains-rustc-parity)
- [2026-09-03 — Prototype commands become integration-grade tools](#2026-09-03--prototype-commands-become-integration-grade-tools)
- [2026-09-03 — Rust codec failures gain stable case identities](#2026-09-03--rust-codec-failures-gain-stable-case-identities)
- [2026-09-03 — Checker corruption becomes a replayable corpus](#2026-09-03--checker-corruption-becomes-a-replayable-corpus)
- [2026-09-03 — Portable C overlays the durable namespace](#2026-09-03--portable-c-overlays-the-durable-namespace)
- [2026-09-03 — The per-file data policy becomes persistent](#2026-09-03--the-per-file-data-policy-becomes-persistent)
- [2026-09-03 — Portable C reads the durable log view](#2026-09-03--portable-c-reads-the-durable-log-view)
- [2026-09-03 — Portable C failures become replayable artifacts](#2026-09-03--portable-c-failures-become-replayable-artifacts)
- [2026-09-03 — The independent portable C reader starts executing](#2026-09-03--the-independent-portable-c-reader-starts-executing)
- [2026-09-03 — FSKit durability moves to write replies](#2026-09-03--fskit-durability-moves-to-write-replies)
- [2026-09-03 — Qualification order follows executable targets](#2026-09-03--qualification-order-follows-executable-targets)
- [2026-09-03 — macFUSE FSKit reopens the host fsync gate](#2026-09-03--macfuse-fskit-reopens-the-host-fsync-gate)
- [2026-09-03 — Logged data fsync reaches the portable Rust adapters](#2026-09-03--logged-data-fsync-reaches-the-portable-rust-adapters)
- [2026-09-03 — Intent-log writes and truncates survive nested recovery crashes](#2026-09-03--intent-log-writes-and-truncates-survive-nested-recovery-crashes)
- [2026-09-02 — Q1 and Q2 architecture blockers closed](#2026-09-02--q1-and-q2-architecture-blockers-closed)
- [2026-09-02 — Shared extents and reflinks qualified](#2026-09-02--shared-extents-and-reflinks-qualified)
- [2026-08-31 — Team board adopted](#2026-08-31--team-board-adopted)
- [2026-08-31 — Shared extents chosen as the next epoch-1 lot](#2026-08-31--shared-extents-chosen-as-the-next-epoch-1-lot)
- [2026-08-31 — Documentation restructured around one home per fact](#2026-08-31--documentation-restructured-around-one-home-per-fact)
- [2026-08-31 — Mountable Alpha-0 closed](#2026-08-31--mountable-alpha-0-closed)
- [2026-08-29 — Region allocator, bounded mount and shared COW trees](#2026-08-29--region-allocator-bounded-mount-and-shared-cow-trees)
- [2026-08-29 — Lesson from the first crash matrix, and the hardening list](#2026-08-29--lesson-from-the-first-crash-matrix-and-the-hardening-list)
- [2026-08-29 — First executable prototype](#2026-08-29--first-executable-prototype)

<!-- /toc -->









## 2026-09-14 - Audit finite executable-core stage requirements

Inspected Stage A's explicit deliverables against block-module exports, trace
accounting and crash/corruption tests. The core and host fault-simulation
components exist. Slice and overlay wrappers are missing from the block surface;
complete artifact replay, CPU/RAM accounting and mutation cache-profile coverage
need stronger evidence or implementation. Recorded these in the status home and
ordered their dependencies in the full audit queue. Stage completion remains
partial for finite work, independently of ongoing review and epoch-1 freeze.

## 2026-09-14 - Preserve scoped symlinks in bound archive groups

[ADR-095](adr/ADR-095-bound-symlink-archive-groups.md) binds captured targets,
exact metadata and full/recovery opaque inventories to symlink namespace groups.
Restoration creates through the destination grant, checks exact target readback
and reports preservation or explicit metadata loss. Real-volume tests cover live
unlink after capture, maximum inline targets, independent tar inspection and
remount. Binary security fixture tests cover full/recovery combinations.

Malformed re-enveloped archives exposed a missing-local-linkpath bug: restoration
could accept the raw tar placeholder as the target. The reader now requires the
explicit local target. Regression cases cover descriptors, profiles, names,
timestamps, kind, mode, ordinal and inventory mismatch, with poisoned replay and
released temporary handles on failure. Full opaque restoration needs a third
slot for the staged upload in addition to parent and created-object handles.

The full all-feature workspace run passed 470 tests with zero failures and ten
ignored qualification tests across 77 suites. Clippy with warnings denied,
formatting, seven documentation fixture tests, documentation and whitespace
checks passed. Whole-job enumeration, graph completeness, durable loss reporting,
source mutation/limit fault expansion and native qualification remain separate
work; this commit does not complete the backup/restore queue gate.

## 2026-09-14 - Separate ongoing review from finite stage completion

The owner asked that moving-target work remain visible without preventing stage
completion. Progress aggregation now excludes explicit Ongoing statuses and keeps
all-ongoing groups uncompleted. Stage 0 is ongoing review. M00 retains its finite
epoch-1 reader-format freeze and contributes to Stage F, with independent decoding
and resolution of experimental fields required before closure.

The early milestone audit records executable evidence and remaining integration
and qualification obligations without changing completion labels. The progress
parser reads only the authoritative milestone table; supplemental audit rows
cannot override it. Seven documentation/discovery/progress fixture tests and the
read-only documentation checker pass. The full filesystem queue remains intact.

## 2026-09-14 — Generate visible milestone and stage progress

Added the requested plain/bracketed/struck notation to planning navigation.
Milestone status cells drive generation; stages aggregate contributing
milestones, while the ongoing Stage 0 review is partial. Prototype completion
with qualification gaps stays partial. The documentation gate detects stale
markers, and `make toc` regenerates them while preserving links and anchors.

## 2026-09-14 - Add scoped symlink transport and VFS dispatch

ADR-094 defines explicit opaque-target operations through the VFS and separate
backup/restore grants. Added optional provider operations with NotSupported
fallbacks, held-grant kind checks, bounded target readback and handle admission
before restore creation. VFS unlink avoids regular-file orphan cleanup and
namespace creation preserves pending logged writes through the existing barrier.

Targeted tests cover live and captured bytes, short buffers, remount, grant
revocation, callback lifetime, unsupported providers, foreign handles and handle
exhaustion. Archive target/profile binding, replacement and native adapters are
separate work; C capability identities are unchanged.

## 2026-09-14 — Refresh implementation navigation and remaining work

Applied the approved docs-refresh review against commit f711cff and the local
working tree. Added phase/stage/milestone navigation; corrected the roadmap's
conditional hybrid-policy and intent-log experiments to ADR-062/063/064/065.
The replaced roadmap text asked to prototype full COW versus private in-place
and add an intent log only after measurement; the accepted ADRs own that
measurement and its decisions. Updated symlink codec and API summaries,
separated snapshot component integration from whole-job backup qualification,
and added snapshot/backup deliverables to the implementation plan.

Source and test inspection establish these corrections. Cargo was unavailable
in this session, so no fresh Rust test result is claimed. Hardware and complete
consumer qualification remain explicit work. Acceptance gates are unchanged.

## 2026-09-14 - Preserve symlink targets through core namespace transactions

Added core symlink creation, bounded live/captured target reads, final unlink,
and payload-preserving metadata and rename rewrites. Explicit metadata decoding
validates the complete inline payload. Directory validation and live/historical
ownership checking accept symlink metadata without treating targets as extents.
Short buffers return the required length and remain unchanged.

Targeted tests cover metadata edits, cross-directory rename, remount, final
unlink, retained snapshots and zero-write refusal. Modeled publication cuts for
create/rename/unlink/restore require complete old or new namespace and metadata,
exact captured bytes and exhaustive checking. Atomic replacement, VFS/backup
capabilities, OS adapters and native qualification retain separate gates.

## 2026-09-14 — Simplify ADR decision statuses

Normalized decision labels and removed approval attribution from ADR headers.
The index links implementation progress and the detailed work queue. Acceptance
is separate from implementation completion and format freeze. Proposed and
superseded decisions retain their lifecycle meaning; ADR-026 is partially
accepted because its public AtomicBatch API is proposed.

Technical qualifications removed from the status labels are retained below as
historical context; their linked decisions and milestone gates govern scope.

- [ADR-007](adr/ADR-007-utf8-nfc.md): Accepted for the executable prototype; epoch-1 interoperability validation pending.
- [ADR-008](adr/ADR-008-case-policy.md): Accepted; volume-default implementation complete, per-directory override pending.
- [ADR-009](adr/ADR-009-journal.md): Architecture resolved by ADR-063; final epoch-1 wire freeze remains M14.
- [ADR-018](adr/ADR-018-inline-data-optional.md): Reopened after PFS4 review.
- [ADR-020](adr/ADR-020-checkpoint-commit.md): Mechanism accepted by ADR-063; final epoch-1 wire and real-device gates remain M14.
- [ADR-022](adr/ADR-022-cache-pinning.md): Accepted as an implementation invariant.
- [ADR-026](adr/ADR-026-bounded-atomic-batches.md): Internal group-commit mechanism accepted by ADR-063; public AtomicBatch API remains proposed.
- [ADR-027](adr/ADR-027-reflink-clones.md): Accepted as an epoch-1 format requirement; implementation may be staged.
- [ADR-028](adr/ADR-028-rust-reference-core.md): Accepted implementation direction.
- [ADR-029](adr/ADR-029-dual-reference-implementations.md): Accepted project policy.
- [ADR-031](adr/ADR-031-portable-security-acls.md): Proposed, split into container-first and semantics-later phases.
- [ADR-034](adr/ADR-034-bounded-cow-tree.md): Accepted for the prototype; four authoritative adapters published.
- [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md): Accepted and authoritative in the executable prototype.
- [ADR-036](adr/ADR-036-reclaim-queue.md): Accepted for the prototype (Reclaim Scale-2); wire format experimental.
- [ADR-037](adr/ADR-037-intent-log.md): Mechanism accepted by ADR-063; record wire remains experimental.
- [ADR-038](adr/ADR-038-mount-policy-and-feature-summary.md): Accepted for the Mountable Alpha-0 prototype.
- [ADR-039](adr/ADR-039-portable-vfs-slice.md): Accepted for Mountable Alpha-0.
- [ADR-040](adr/ADR-040-fuse-protocol-boundary.md): Accepted for Mountable Alpha-0.
- [ADR-041](adr/ADR-041-aros-dos-adapter.md): Accepted for Mountable Alpha-0.
- [ADR-042](adr/ADR-042-aros-c-boundary.md): Accepted for Mountable Alpha-0.
- [ADR-043](adr/ADR-043-native-aros-dospacket-translator.md): Accepted for Mountable Alpha-0.
- [ADR-044](adr/ADR-044-aros-trackdisk-viewport.md): Accepted for Mountable Alpha-0.
- [ADR-045](adr/ADR-045-native-aros-handler-shell.md): Accepted for Mountable Alpha-0 integration.
- [ADR-046](adr/ADR-046-hosted-aros-same-image.md): Accepted for Mountable Alpha-0 S0.
- [ADR-047](adr/ADR-047-hosted-aros-crash-replay.md): Accepted for Hosted and native-QEMU qualification.
- [ADR-048](adr/ADR-048-hosted-aros-system-pivot.md): Accepted; S1a and Hosted S1b qualified.
- [ADR-049](adr/ADR-049-hosted-aros-desktop-pivot.md): Accepted for Hosted S1b.
- [ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md): Accepted for Hosted and native-QEMU lifecycle.
- [ADR-051](adr/ADR-051-explicit-aros-aarch64-platform-profiles.md): Accepted; apple-aarch64 pre-hardware runtime profile qualified.
- [ADR-052](adr/ADR-052-versioned-directory-comparison-keys.md): Accepted for the executable prototype.
- [ADR-053](adr/ADR-053-native-macaros-retained-image-transport.md): Accepted for pre-hardware QEMU qualification.
- [ADR-054](adr/ADR-054-native-macaros-crash-replay-extraction.md): Accepted for native MacAROS pre-hardware qualification.
- [ADR-055](adr/ADR-055-aros-m68k-emulator-gate.md): Accepted for pre-hardware qualification.
- [ADR-056](adr/ADR-056-native-aros-m68k-alpha0-and-replay.md): Accepted for the M68020-or-newer emulator reference profile.
- [ADR-057](adr/ADR-057-plain-m68000-emulator-gate.md): Accepted for A500-configured emulator functionality and recovery.
- [ADR-058](adr/ADR-058-m68000-memory-and-restart-lifecycle.md): Accepted for the A500-configured emulator profile.
- [ADR-059](adr/ADR-059-guest-failure-diagnostics-are-gate-verdicts.md): Accepted for all current AROS runtime gates.
- [ADR-061](adr/ADR-061-shared-extent-references.md): Accepted; wire format experimental until M14.
- [ADR-062](adr/ADR-062-explicit-hybrid-data-updates.md): Accepted as the epoch-1 data-update architecture; the persistent policy encoding is assigned by ADR-065.
- [ADR-063](adr/ADR-063-intent-log-epoch1.md): Accepted as the epoch-1 durability architecture; intent-log wire remains experimental.
- [ADR-064](adr/ADR-064-intent-log-data-update-compatibility.md): Accepted for the experimental version-3 record set; final wire freeze remains M14.
- [ADR-065](adr/ADR-065-persistent-data-update-policy.md): Accepted; wire format experimental until M14.
- [ADR-066](adr/ADR-066-bounded-orphan-directory.md): Accepted; wire format experimental until M14.
- [ADR-067](adr/ADR-067-epoch1-allocation-state.md): Accepted; exact wire format remains experimental until M14.
- [ADR-069](adr/ADR-069-consistent-snapshots-first.md): Accepted direction; retention policy and on-disk representation require further decisions.
- [ADR-070](adr/ADR-070-persistent-snapshot-priority.md): Accepted direction; persistent representation and retention policy require prototype evidence.
- [ADR-071](adr/ADR-071-snapshot-lifetime-prototype.md): Accepted for the integrated experiment; shipping wire and resource qualification open.
- [ADR-072](adr/ADR-072-snapshot-record-codecs.md): Accepted for the ADR-071 prototype; integration and wire freeze require qualification.
- [ADR-073](adr/ADR-073-snapshot-checkpoint-roots.md): Accepted for the ADR-071 prototype; writable snapshot support requires integration.
- [ADR-080](adr/ADR-080-pax-completion-envelope.md): Superseded by ADR-081 after independent Python tarfile recovery rejected the terminal global header.

## 2026-09-14 - Cross-read inline symlink records with Rust and C

Accepted ADR-068 for experimental implementation under delegated design authority,
with its full integration and platform qualification requirements retained. Added
an explicit borrowed-target Rust codec and an independent heap-free C record
validator. Fixed-record operations reject symlinks until mutation paths preserve
the variable payload; the standalone C function does not enable volume writes.

Strict and sanitized C probes cross-read relative, absolute, AROS-qualified,
Unicode and maximum-length Rust records and reject fourteen malformed variants
per target. Rust also checks every undersized encoding buffer, truncated input,
reserved fields and nonzero unused tails. Complete namespace mutations, snapshot
reads, adapters and platform qualification remain separate implementation work.

## 2026-09-14 - Reject undersized object encoder buffers

Reviewing the fixed record before the symlink extension found that an empty
file encoded into a caller-selected buffer shorter than 128 bytes could panic
while slicing the common header or fixed fields. The regression reproduced the
zero-length panic before the fix. The encoder checks the fixed header/payload
minimum before allocation or field writes and returns WrongBufferSize.
The test checks every size from 0 through 127 and round-trips the exact minimum,
one byte above it and the ordinary block size. No valid wire representation
changes. Complete symlink codecs and payload-preserving namespace/metadata
rewrites retain their separate implementation gates.

## 2026-09-14 - Bind directory and alias archive restoration

Added ADR-093 directory and hard-link groups to the archive consumer. Directory
restoration checks scoped emptiness, preserves full inventories or explicitly
reports recovery losses, and verifies exact metadata. Alias restoration checks
the primary descriptor and destination capacity before linking, then verifies
shared identity and incremented link count. Added the optional grant-held
one-entry emptiness query without creating another active handle.

Qualification covers a real snapshot directory/file/alias archive, live mutation,
Python tarfile inspection, bounded handle reopening, final directory metadata,
remount, shared writes, malformed bindings, revoked grants and resource refusals.
Full-directory opaque preservation uses a semantic provider; AFS+ opaque storage,
symlinks, complete namespace planning and durable job reports are separate gates.
The specification, API contract, qualification plan and queue preserve that scope.
The full workspace suite passed with 453 tests and 10 ignored qualification
workloads. Formatting, Clippy and documentation checks passed.

## 2026-09-14 - Reconcile the filesystem comparison with executable support

The owner requested an implementation-aware comparison and a personal “Built
by me” row checked only for AFS+. Replaced stale planned labels for implemented
core capabilities with explicit prototype support, linked code and qualification
evidence, and distinguished partial APIs and constrained profiles from complete
platform qualification. Unimplemented facilities retain planned/proposed labels.
Corrected the accelerator discussion: lost change history requires a reset and
rescan, not reconstruction. No filesystem behavior or format changed.

## 2026-09-14 - Reopen created restore entries with bounded active handles

ADR-092 used delegated recommended-option authority to add scoped
`lookup_created`. Namespace restoration needs to revisit files for aliases and
metadata finalization; requiring every handle to stay open would tie active
resource usage to the full object count. The new operation resolves one literal
component in the initially empty, exclusively owned restore destination, holds
the original grant through parent validation and lookup, and uses the existing
handle budget. It neither follows symlinks nor imports arbitrary object IDs.

The semantic fixture walked and reopened 128 levels with two active slots.
Invalid names, exhausted slots and foreign or revoked grants prevented provider
calls. Non-directory and symlink parents stopped before lookup; failed lookups
released their slot. A fresh authorized grant reopened an in-scope entry without
reactivating old revoked handles. The operation-permit probe covered both parent
stat and the actual provider lookup.

The real AFS+ fixture reopened eight nested directories and hard-linked names,
finalized exact file/directory metadata after namespace edits and verified bytes,
identity and metadata after remount. The outside file was untouched and could
not be reached from the restore root. Each measured lookup issued zero writes
and barriers and fewer than 128 reads in this fixture.

The full workspace all-features gate passed 445 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

This is a live isolated-job operation, not pre-existing-destination merge,
overwrite or persistent resume. Native providers must independently qualify
exclusive ownership and no-follow behavior. Archive indexes/path storage and
complete namespace/job orchestration remain separate bounded-resource work.

## 2026-09-14 - Bind exact regular-file metadata to allocation and opaque inventories

ADR-091 used the delegated recommended-option authority to define explicit full
and recovery regular-file groups. The group binds exact metadata to allocation,
sparse contents and optional complete opaque inventories. Full restore refuses a
recovery group before writes. Recovery of a full group validates and drains its
opaque values without destination uploads and reports their counts/bytes and
captured knowledge. Uninspected inventories remain explicit uncertainty.

The real AFS+ snapshot fixture recovered captured bytes, sparse length,
protection and three distinct nanosecond timestamps through remount, despite
later live-source writes. It used 512-byte transfers and one-entry pages and
reported omitted reservations and uninspected inventories. A separate semantic
provider fixture preserved full-width protection, signed timestamp extremes and
unknown binary security values with one-byte transfers. These are different
evidence scopes: the latter does not establish AFS+ opaque storage support.

Rebuilt valid envelopes with conflicting file paths, kinds, mtime, inventory
knowledge/digests and value bindings were refused. Timestamp rounding at the
destination withheld success. Revoked authority, resource limits, mode conflicts,
unknown versions and source export failures poisoned further archive use. Core
metadata was applied after contents/inventories, then read back exactly. Existing
opaque parsing was shared with the recovery drain, with raw/effective ordinal
checks and fallible admitted record allocation.

The full workspace all-features gate passed 441 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

The no-default-features archive gate passed 33 tests. Namespace orchestration
must still preserve directories, symlinks and hard-link aliases, finalize metadata
after namespace edits, validate whole-job completeness, synchronize and persist
loss reports. Native/older-system resource and durability gates remain separate.

## 2026-09-14 - Bind archive allocation records to verified sparse restoration

ADR-090 used the owner's delegated recommended-option authority to bind a
versioned allocation record to its sparse file. The exporter derives both from
one captured plan. Restore validates path, size, ordinal and clipped written
coverage before writes, then reserves in bounded calls and compares committed
destination coverage. Equivalent extent splitting is accepted; changed holes or
written/unwritten state is refused. Explicit recovery skips reservations and
returns their discarded range count and byte sum. The simpler content-only
export path avoids retaining the additional allocation list.

Targeted tests passed captured-source remounts and both restore modes at zero,
1 TiB-plus and maximum-u64 logical lengths, with reservations inside/beyond EOF
and in the final address block. A separate hand-constructed archive exercised
rounded written tails. Unsupported providers, changed reservation locations,
conflicting records/maps, revoked grants and budget exhaustion withheld success;
errors poisoned further archive use. The remount oracle compared independent
logical block/state maps and captured bytes. Codec tests rejected every
truncation, malformed fields/ranges and admitted-limit violations.

The full workspace all-features gate passed 437 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict workspace Clippy,
documentation, three checker fixtures and whitespace validation passed.

Python and libarchive sparse extraction passed, including hole preservation and
independent wide-header decoding. Those generic-tool checks establish content
recovery, not reservation preservation. Whole-object metadata/identity binding,
job completion/loss-report persistence, native durability and actual older-system
peak-memory qualification remain separate work. Explicit quotas, small pages and
bounded transfers provide the constrained implementation path without silently
weakening preservation semantics.

## 2026-09-14 - Add scoped allocation readback for full restore verification

ADR-089 chose explicit destination allocation readback before accepting full
reservation preservation. A successful reservation call or matching total byte
count cannot identify changed holes or written/unwritten coverage. The new
restore query holds the original object's revocable grant, validates bounded
page responses and refuses missing provider support. The AFS+ query shares the
bounded extent reader with captured snapshots and adds no format or C ABI change.

Targeted tests passed one-entry enumeration, written data, beyond-EOF
reservations and a final range ending at 2^64, with exact layout after remount.
They rejected malformed pages, invalid budgets, foreign services and revoked
grants. The operation-permit probe covers the query itself. Core tests proved
zero query writes/barriers for empty, direct and tree layouts, and showed that
live allocation changes leave captured layouts unchanged. An open log window
caused explicit refusal without flushing; an explicit window commit made its
new layout available. Ordinary sync intentionally does not commit an open window.

The full workspace all-features gate passed 430 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

This supplies verification evidence for the allocation-preserving archive
consumer. The archive allocation record, binding to sparse data, reservation
restore ordering and semantic coverage comparison remain the next integration
work. Full job and native/older-system qualification remain separate.

## 2026-09-14 - Transport captured sparse contents through the archive consumer

ADR-087 selected GNU sparse PAX 1.0 rather than expanding holes or replacing
ordinary recovery with a private content container. Research used the GNU tar
1.35 manual's sparse-format section, accessed on 2026-09-14. The exporter reads
semantic allocation pages through the retained snapshot grant; the importer
validates a bounded map before writing through the separate destination grant.
Written zeros stay data. Omitted unwritten reservations have explicit range and
byte totals for the enclosing content-recovery loss report.

An independent Python 3.9.6 test exposed conflicting PAX `size` behavior:
applying stored length after sparse metadata destroyed logical length and
following-member alignment. ADR-088 moved stored size into the raw header and
refused the conflicting override. Positive GNU binary size encoding handles the
33-bit octal boundary through unsigned 64-bit maximum without imposing a format
limit at 8 GiB. Python's independent numeric codec matched those headers. These
are representation tests, not multi-gigabyte payload qualification.

The sparse interop script passed Python 3.9.6 and bsdtar 3.5.3/libarchive 3.7.4
for mixed, all-hole and empty entries, including following-member alignment and
non-expanded all-hole extraction. The real AFS+ service test captured a file of
1 TiB plus 23 bytes, changed and shrank the live source after capture, and
restored the original content through verified replay into an isolated AFS+
destination. After synchronization and remount, exact written bytes, sampled
holes, logical size, written allocation and an outside file matched the oracle.
The archive was below 16 KiB, with 8192 written bytes and an 8704-byte sparse
payload. Export issued no source writes/barriers and fewer than 256 block reads.
The report identified two omitted unwritten ranges totaling 8192 bytes.

The workspace all-features gate passed 424 tests with zero failures and 10
explicitly ignored qualification probes. The default archive crate passed
33 tests. Formatting, strict Clippy, documentation, three checker fixtures,
whitespace validation and independent sparse extraction passed.

Targeted cases also passed source revocation during output for data and all-hole
files, wrong path/revoked/nonempty destination refusal, multi-block maps,
maximum logical length, malformed/count/overlap/overflow/padding/size cases,
resource admission and every truncated archive prefix. Ordinary constructors
explicitly refuse sparse fields; opt-in spool verification certifies integrity,
with semantic map validation at the consumer. Full reservation sidecars, object
metadata, namespace/link/inventory binding, complete-job outcomes, spooled large
maps and native resource qualification retain their queue gates.

## 2026-09-14 - Exercise repeated low-space reclamation with retained snapshots

The 24-cycle near-full fixture was extended to persistent snapshots in the
core snapshot harness. Both one-record and eight-record lifetime scan profiles
preserved exact original, rotating-view and live bytes through every remount.
Both selectable checkpoints and their snapshot registries passed exhaustive
verification each cycle. Oversized allocations issued zero writes/barriers;
active reader handles refused snapshot deletion. Every cycle recovered more
than 384 normally available blocks on a 512-block image within 512 calls.

The full workspace all-features gate passed 416 tests with no failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.

The targeted debug memory-backend run measured these maintenance-only totals:

| Scan budget | Calls | Records scanned | Blocks promoted | Reads / writes | Bytes read / written | Barriers |
|---|---:|---:|---:|---:|---:|---:|
| 1 | 418 | 418 | 10,156 | 9,132 / 2,508 | 37,404,672 / 10,272,768 | 836 |
| 8 | 185 | 1,174 | 9,880 | 4,069 / 1,047 | 16,666,624 / 4,288,512 | 370 |

Each profile admitted all 24 cross-block writes and reached a minimum of 44
free blocks. Maintenance bitmap payload peak was 64 bytes, excluding all other
memory. Local maintenance elapsed times were about 0.92 and 0.42 seconds; these
are debug algorithm timings, not storage latency or a performance promise.
The smaller record budget preserved correctness while increasing calls and I/O.
No format change or hardware write was involved. Sustained aged workloads,
full process memory and native durability/resource gates stay explicit.

## 2026-09-14 - Bind full object inventories to counted descriptor manifests

Under the owner's delegated authority, ADR-086 chose a two-pass descriptor
manifest rather than retaining every key in memory. Counts, byte totals and
ordered SHA-512/256 descriptors bind each inspected object inventory. Export
refuses unknown inventory knowledge and poisons completion on source changes.
Verified scratch import enforces path, class, key and ordinal binding with one
upload slot, refusing success and further reader use after any discrepancy.
Earlier complete publications are explicitly partial restoration, not rollback.

The full all-features workspace gate passed 415 tests with zero failures and
10 explicitly ignored qualification probes. Formatting, strict Clippy,
documentation, three checker fixtures and whitespace validation passed.
The archive component's 45 tests passed. Inventories of 0, 1 and 70 entries
round-tripped every key and byte with pages of 1, 3 and 64 entries, one-byte
transfer buffers and 512-byte scratch chunks. Descriptor calls were exactly two
complete enumerations. Failure cases covered altered hashes/counts/byte sums,
duplicate keys, class/path mismatch, revoked grants, unverified replay,
uninspected sources, changing descriptors and admission/ordinal boundaries.

This is a full-preservation inventory component, not whole-job completion.
Cross-object namespace/data/reservation binding, AFS+ opaque storage mapping,
content-recovery loss reporting and native/older-system measurements retain
their queue gates. Small buffer fixtures establish hosted resource behavior;
they do not establish a working m68000 or bare-metal backup service.

## 2026-09-14 - Replay verified scratch archives with bounded upload slots

Accepted ADR-085 under delegated recommended-option authority. Added quota-bound
scratch capture, a privately retained Merkle root and checked replay. Capture
verifies the archive through chunk proofs before returning a replay capability.
Each replayed chunk is checked before exposing bytes; no caller-supplied root or
scratch mutation handle is exposed. Verified replay permits one staged value to
publish and release its upload slot before the next, while ordinary streaming
input retains its final-EOF gate. Reader identity and failure checks remain.

The integrated test restored 20 distinct keys with only root plus one upload
slot. Other tests compare an independently calculated Python SHA-256 root,
exercise odd trees and partial chunks, reject altered/locally resealed/reordered
bytes and proof hashes, enforce quotas and I/O failure handling, and replay an
actual temporary regular file. The complete workspace passed 410 tests with
zero failures and ten ignored tests; the archive consumer passed 40 tests.
Formatting, workspace Clippy, documentation checks, three checker fixtures and
whitespace validation passed.

The 3,584-byte fixture used 4,032 scratch bytes with 512-byte chunks and four
levels: capture plus verification/replay made 68 reads/8,896 read bytes and
21 writes/4,032 write bytes. With 4,096-byte chunks it used 4,128 scratch bytes,
one level, three reads/8,224 read bytes and two writes/4,128 write bytes. Maximum
read requests equaled one chunk in each case. These are small host resource
oracles, not sustained workload or whole-process RSS measurements. Full inventory
validation, segmented scratch beyond host seek limits, AFS+ metadata storage and
native lifecycle qualification remain owned queue work.

## 2026-09-14 - Carry opaque values through an authorized archive consumer

Accepted ADR-084 under delegated recommended-option authority. Added bounded
PAX descriptor/binary pairs with canonical ordinal and exact source-path, class,
key, encoding and size binding. Export reads through the captured backup reader,
checks exact length/EOF and poisons completion after any failure. Import opens a
staged destination upload only after pair validation. Its staged result can
publish only when that exact archive reader verifies the terminal digest and EOF;
a receipt from another reader cannot release it. This avoids activating a value
before later archive corruption is detected.

End-to-end tests cross independent source and destination providers using two-
and three-byte buffers, preserve unknown binary and empty values after live
source mutation, and check exact identities. Wrong bindings, source-length errors,
revocation, corrupted bytes, premature/foreign-reader publication and every
truncated archive prefix leave active metadata absent and release staging.
Descriptor tests cover required fields, versions, duplicates, limits and scalar
boundaries. The consumer dependency is optional; standalone framing/metadata
codecs do not require VFS. Updated the spec, queue and navigation.

Validation passed: 403 workspace tests, zero failures, ten ignored tests;
33 archive tests with the consumer and 28 standalone tests; formatting, workspace
Clippy, documentation checks, three checker fixtures and whitespace validation.
This qualifies an opaque-value transport component, not the whole backup job.
Complete inventory/object matching, large-inventory staging/spooling, AFS+ opaque
storage, generic metadata recovery and native resource/durability gates remain.

## 2026-09-14 - Stage opaque destination metadata before atomic publication

Accepted ADR-083 under delegated recommended-option authority. Added an optional
provider extension and non-cloneable upload handle bound to the original object
lease and grant. Host value admission defaults to disabled; each upload also
consumes a handle-budget unit. Sequential chunks remain in private staging until
exact-length finish. Staging errors permanently fail the upload; dropping,
revocation or premature finish releases provider staging without publishing.
The AFS+ mapping explicitly refuses the extension pending storage qualification.

Independent-provider tests cover exact unknown binary and empty values,
pre-finish invisibility, object-lease retention, budget recovery, invalid/excessive
requests, foreign/revoked grants, partial staging-write faults and old/complete-new
publication-error outcomes. In-backend probes verify held admission for every
phase. Remounted AFS+ refusal issues zero writes and flushes. Updated API,
qualification and queue ownership; accepted ADR bodies are unchanged except for
amendment relations.

Validation passed: 398 workspace tests, zero failures, ten ignored tests;
43 VFS tests including doctests; formatting, workspace Clippy, documentation
checks, three checker fixtures and whitespace validation. These are host
memory-provider tests. Durable AFS+ metadata storage, staging cleanup recovery,
archive binding and complete constrained/native qualification remain queue work.

## 2026-09-14 - Stream opaque captured attributes and security metadata

Added checked metadata descriptor pages and caller-buffer value reads to the
snapshot service and consumer facade. Attribute/security channels stay distinct;
keys and encoding identifiers remain exact and unknown binary values are not
interpreted. Requests and returned descriptors have explicit size, ordering and
progress checks. Missing implementations refuse transport. Operation permits
cover provider execution, and revoked calls preserve output buffers.

Independent-provider tests enumerate one entry at a time and read in two-byte
chunks, including empty, UTF-8 and unknown binary values. Captured data survives
live inventory changes. Malformed requests/results, cursor nonprogress, invalid
read counts, revocation and unsupported remounted AFS+ behavior are covered.
The API documentation records additive Rust compatibility and separate native
and destination requirements.

Review also found that docs/30 described canonical ACL evaluation as settled,
contradicting Proposed ADR-031 and unresolved Q5. Clarified its candidate scope,
retained the candidate design for evaluation, and linked the preservation and
qualification owners. This does not accept rich ACL evaluation semantics.

Validation passed: 394 workspace tests, zero failures, ten ignored tests;
39 VFS tests including doctests; formatting, workspace Clippy, documentation
checks, three checker fixtures and whitespace validation. Source transport is
not complete backup/restore: archive binding, lossless destination installation,
AFS+ metadata storage and actual native provider support remain queue work.

## 2026-09-14 - Inspect captured metadata knowledge through backup authority

Added a filesystem-neutral fixed-size inventory-knowledge operation to the
trusted provider, service and consumer facade. The conservative default validates
the captured object through stat and reports uninspected attribute/security
inventories. A complete provider may report empty or present from its captured
view. Admission holds the original reader grant throughout inspection, with
wrong-service and revocation rejection before backend access. Documented additive
Rust compatibility and explicit C/native qualification boundaries.

The independent provider test preserves captured inventory knowledge after live
changes. Default-provider tests verify held authority, missing-object errors,
revocation and fresh-grant isolation. The AFS+ remount fixture verifies
uninspected results with zero writes/flushes and a clean checker. Validation
passed: 392 workspace tests, zero failures, ten ignored tests; all 37 VFS tests
including doctests; formatting, workspace Clippy, documentation checks, three
checker fixtures and whitespace validation. Actual attribute/security enumeration
and transport remain queue work; this result does not certify full preservation.

## 2026-09-14 - Preserve exact object metadata and inventory knowledge

Accepted ADR-082 under the owner's delegated recommended-option authority.
Inspection found no attribute/security enumeration in the authorized snapshot
API; treating that limitation as an empty inventory would misstate preservation.
The versioned object payload therefore records empty, present or uninspected
inventories explicitly, alongside exact protection, three timestamps, path and
kind. Added a bounded borrowing codec with required-field, numeric, namespace,
duplicate, truncation and inventory-state regressions. Updated the specification,
Q11, queue dependency and documentation indexes. The accepted ADR-076 body is
unchanged; its amendment relation links the new decision.

Validation passed: 390 workspace tests, zero failures, ten ignored tests;
28 archive tests; formatting, workspace Clippy, documentation checks, three
checker fixtures and whitespace validation. No filesystem disk record or ABI
changed. Complete inventory transport, archive-wide matching and authorized
restoration remain implementation work; codec success cannot close those gates.

## 2026-09-14 - Bind local PAX records to streamed ordinary members

Integrated effective-field admission with envelope reading. Local records apply
exactly once; stacked or dangling blocks fail permanently. Metadata bytes are
bounded before buffer allocation and effective size selects framing before
payload access. Caller buffers stream contents, early advancement reports busy,
and a completion receipt cannot bypass local-record failure. Added exact
one-member override and payload oracles, admission/poisoning checks and every
truncated archive prefix. Full preservation and actual restoration remain
separate queue gates.

Validation passed: 387 workspace tests, zero failures, ten ignored tests;
25 archive tests; formatting and workspace Clippy; documentation checker,
three checker fixtures and whitespace validation. No filesystem record or
native ABI changed, and no hardware qualification is inferred.

## 2026-09-14 - Validate effective PAX member fields before restoration

Added borrowed ordinary-member resolution with explicit PAX record admission,
resolved namespace and hard-link checks, strict numeric overrides, unsupported
metadata refusal and exact signed nanosecond conversion. Reused the envelope's
canonical-path rule and extracted record preflight without serializing another
payload buffer. Added four regression tests covering override escapes, links,
resource admission and timestamp/numeric boundaries. The contract retains
numeric and textual identities separately; host account mapping and complete
preservation semantics belong to the consumer.

Validation: all 384 workspace tests passed, zero failed and ten were ignored.
The archive crate passed 22 tests. Formatting, workspace Clippy, documentation
checking, three checker fixtures and whitespace validation passed. This is host
admission evidence, not native or older-hardware qualification. Full sparse,
attribute/security transport and integrated archive restoration remain queue
work; no milestone was closed by these helper tests.

## 2026-09-14 - Bind archive integrity and termination to an independently checked envelope

Accepted [ADR-080](adr/ADR-080-pax-completion-envelope.md) under delegated
recommended-option authority, selecting SHA-512/256 and 128-bit byte accounting
after the NIST message-domain review exposed SHA-256's smaller bit-length limit.
RustCrypto sha2 0.11.0 supplies the implementation; seven packages were added to
the lockfile. The digest is unkeyed and makes no authentication claim.

Independent recovery rejected the terminal global-header prototype: Python
3.9.6 tarfile required a subsequent member. [ADR-081](adr/ADR-081-ordinary-pax-completion-member.md)
superseded it with version 2, a regular terminal control file and separated
`files`/auxiliary namespaces. The accepted earlier decision remains intact as
history. Source names resembling control names can live under the source subtree.
The bundled Python/LibreSSL lacked SHA-512/256; the gate uses existing OpenSSL
3.6.4 independently, without adding a runtime dependency on that executable.

Eighteen archive-crate tests passed on both default and compact software hash
backends. They cover delayed receipts, exact counts/digests, truncated prefixes,
changed body/control bytes, unsupported versions, duplicate/missing completion,
path/control collisions, limits and failed output/flush. Review extended the
raw-name guard to local PAX header names, and its regression cases pass on both
backends. Independent OpenSSL
hashes and Python/bsdtar ordinary recovery passed for both backends, including a
changed-payload negative oracle. Formatting, Clippy, documentation checks,
three checker fixtures and whitespace validation passed. The full workspace
suite passed 380 tests, with zero failures and ten ignored.

Profile interpretation, opaque metadata transport, actual authorized snapshot
and restore consumers, performance/peak RAM and native/older-system execution
remain queue requirements. The receipt establishes stream integrity and actual
termination; it does not establish full preservation or authenticated provenance.

## 2026-09-14 - Add streamed tar framing and independent recovery checks

Added strict ustar headers and a caller-buffer streaming reader/writer.
Member counts, effective payload sizes and trailing zero blocks have explicit
limits. The profile must validate PAX overrides before body selection. Invalid
headers/padding, truncated bodies/end markers and uncertain I/O poison the
reader/writer; successful finish establishes framing, with profile completion
and destination durability as separate requirements.

Eleven crate tests passed, including six framing tests: independent Python
fixture decoding, every truncated Rust-archive prefix, valid-checksum malformed
headers, long UTF-8 names, limit/sequencing checks, 64-bit override arithmetic,
and partial write/padding/finish/flush/read failures. The independent host gate
passed with Python tarfile and bsdtar 3.5.3 / libarchive 3.7.4, exact ordinary-file
payloads, Python header metadata, reproducible fixture bytes and a changed-byte
negative oracle. It extracted only to a private temporary buffer/file.

Formatting, Clippy, documentation checks, three checker fixtures and whitespace
validation passed. The full workspace suite passed 373 tests, with zero failures
and ten ignored. Sparse/full-metadata interoperability, the preservation profile,
integrity/completion, real authorized consumers and constrained/native runtime
remain integration requirements; ordinary tar success does not close them.

## 2026-09-14 - Add bounded PAX record parsing for the archive consumer

Added the dependency-free host archive crate and strict unique-key UTF-8 PAX
record codec. Explicit byte, record, keyword and value limits bound admission;
decoding borrows payload strings and encoding preflights total output bytes.
Duplicate keys, malformed lengths, invalid UTF-8 and NUL are rejected. Empty
values are retained for profile interpretation; general override semantics
and archive completion belong to the next archive layer.

Research checked the Oracle Solaris pax manual's extended-header section:
[primary source](https://docs.oracle.com/cd/E88353_01/html/E37839/pax-1.html).
The current Open Group endpoint returned HTTP 403, and GNU direct-page fetches
failed, so those fetches supply no new verification. Python 3.9.6 tarfile
independently generated the retained UTF-8/newline/equal-sign timestamp fixture;
exact encoder and decoder equality passed. No private project details entered
public queries.

Five tests passed, covering the golden record block, decimal-width boundaries,
every nonempty truncated single-record prefix, exact error classes, duplicate
keys, limits in both directions, and deterministic arbitrary input. Formatting,
Clippy, documentation checks, three checker fixtures and whitespace validation
passed. The full workspace suite passed 367 tests, with zero failures and ten
ignored.

The archive codec does not prove a complete backup. Framing, the versioned
preservation profile, integrity/completion, attribute/security transport,
independent archive recovery and real snapshot/restore consumers remain explicit
integration requirements in the queue and archive qualification plan.

## 2026-09-14 - Bound shrink working sets and retirements

The host Rust bounded resize entry point admits affected tail records and
retired allocated blocks. A replaced partial written tail counts once;
reservations beyond EOF count toward retirement, and holes do not. Local tree
edits preserve unrelated mappings and may retain an empty tree. Shared data
and captured views use the existing reference/lifetime retirement machinery.
The restore provider admits 64 records and 64 retired blocks per resize,
refusing excessive work before writes without silently splitting the operation.

Three targeted tests passed. A partial-tail edit among 301 fragmented records
used 99 device reads and preserved snapshots through shrink/regrowth/remount.
Record/block refusals issued zero writes. Empty-tree truncation preserved its
shared peer. The publication oracle passed 1171 states (1167 old, four new),
checking exact live records/bytes, shared-peer content and captured accounting.

Formatting, Clippy, documentation checks, three checker fixtures and whitespace
validation passed. The full workspace suite passed 362 tests, with zero
failures and ten ignored. Arbitrary large atomic truncation requires persistent
bounded cleanup and its own crash
oracles. Total RAM and native/constrained runtime need separate qualification.
Those requirements remain in the complete queue.

## 2026-09-14 - Grow sparse files without enumerating mappings

Extent-tree growth retains the exact root and allocation accounting while
publishing size, times and content generation through the existing COW tail.
The fixed-size direct path retains its promotion rules. A 300-record file
required 51 device reads to grow to the maximum logical size, preserving its
captured view and final-block zero reads through remount.

The maximum-size regression exposed an overflowing read block-end addition.
Saturating the boundary before EOF clipping fixes it; restoring the former
expression reproduced the overflow as a negative control. Growth publication
passed 346 modeled crash states (342 old, four new), checking exact object
records, complete bytes, reservation accounting and snapshot contents.

The targeted tests, formatting, Clippy, documentation checks, three checker
fixtures and whitespace validation passed. Full workspace regression passed
359 tests with zero failures and ten ignored. Bounded shrinking, total RAM and native/constrained runtime
gates remain separate requirements in the queue.

## 2026-09-14 - Bound atomic writes for restore consumers

Added the host Rust bounded-write entry point, using local extent windows and
the generalized changed-key publication helper. It admits touched blocks and
local records, preserves existing tree representation, and shares the data,
reference and checkpoint publication machinery. The restore provider uses
64-block and 64-record limits; larger transfers require smaller independently
durable requests. Ordinary unrestricted writes retain their admission behavior.
No disk encoding, C ABI or native capability advertisement changed.

Four targeted tests passed: a 130-record fragmented byte oracle with snapshot
and remount checks; block/record refusal before writes; shared-peer isolation;
empty/direct promotion; eligible private tree overwrites with unchanged physical
mappings; and 642 publication crash states (638 old, four new) preserving
captured zeros. The full workspace suite passed 357 tests with zero failures
and ten ignored. Formatting, Clippy, documentation checks, three checker
fixtures and whitespace validation passed.

Total peak RAM, allocator/retention bounds, truncation memory, sustained
near-full operation and native/constrained runtime qualification need further
work. The local-vector and data-buffer limits do not close those queue gates.

## 2026-09-14 - Bound reservation edits in fragmented files

The host Rust reservation entry point accepts explicit touched-block and
extent-record budgets. Local key pages and changed-key COW edits avoid loading
and rebuilding unrelated extent mappings. The checked restore provider uses
64 records plus its existing host-granted byte bound. Refusal occurs before
device writes; callers can retry smaller separately durable requests. The
convenience core API preserves unrestricted request admission. No disk, C ABI,
or native advertisement changed.

The full workspace suite passed 353 tests, with zero failures and ten ignored.
Formatting, warning-denying Clippy, documentation checks, three checker fixtures
and whitespace validation passed.

Three targeted tests passed. A one-block reservation among 600 fragmented
records used 98 device reads, 12 metadata writes, 61440 written bytes and two
flushes. Boundary/no-op and input/result-budget tests preserved exact records
and issued zero writes on refusal. The multi-node publication oracle passed
4300 crash states: 4296 old and four new, with exact live mappings and captured
allocation ranges. Full graph checking covered selectable checkpoints.

The design bounds local extent vectors; total peak RAM and before/after process
memory measurements are unqualified. Ordinary writes/truncation and allocator
working sets need their own bounds. Native and older-system execution remains
UNVERIFIED. These gates stay in the complete queue.

## 2026-09-14 — Cross-read initialized reservations through the portable C reader

Added deterministic baseline-format images before and after private reservation
initialization, plus a fallback image whose newer checkpoint is invalidated.
The fallback keeps the changed physical payload while the older mapping must
return logical zeros. A separate C probe verifies all 16,384 logical bytes in
257-byte chunks. Strict C99 and ASan/UBSan runs passed the three images, with
75/108/75 callback reads; negative controls rejected initialized data when zeros
were expected.

The probe uses 6 KiB caller scratch for directory traversal and 4 KiB for object
lookup/file reads; a smaller lookup workspace is refused before I/O. These
bounds exclude stack, stdio and process memory. The configured/default m68k
compiler path was unavailable, so the optional compile gate reported SKIP;
m68000 runtime and total-resource qualification remain unverified. The fixture
has no snapshot-registry feature, keeping that portable consumer gate separate.

Validation: 350 workspace/all-features tests passed, zero failed and 10 were
ignored. Strict and sanitized C reads passed six image cases plus two negative
controls. Formatting, Clippy, shell syntax, documentation, three checker
fixtures and whitespace checks passed. The m68000 compile gate explicitly skipped.


## 2026-09-14 — Initialize private unwritten reservations without replacement data allocation

Under the owner's delegated design authority, ADR-079 selects initialization
of proven-private unwritten blocks followed by COW publication of their written
mappings. The baseline regression reproduced replacement allocation (physical
block 25 became block 33). The implementation preserves eligible physical
addresses and lifetime birth, checks the live reference tree, and keeps shared
or stale-marked reservations on fresh storage. A mixed write copies written
portions while initializing eligible reservations. Commit counters distinguish
initialization from the opted-in written-data in-place policy.

Six targeted tests passed: physical reuse in mixed writes, shared/stale-marker
fallback, low-space progress, 362 crash states (358 old and 4 complete new),
false-private shared-reference refusal, and completed-write/three-barrier I/O
failures. The low-space fixture initialized 32 data blocks with 24 blocks
available for new allocation, writing 5 metadata blocks and 163,840 bytes with
3 flushes while the snapshot retained zeros. Both checkpoints passed ownership
checking. These are host fixtures; portable C cross-reading, bounded fragmented
layout traversal, sustained pressure and older-target resource evidence remain
explicit work.
Also corrected a stale runtime-only qualification sentence in files/extents;
ADR-065 and the implemented persistent policy remain the authority.

Validation: the workspace/all-features suite passed 350 tests, zero failures
and 10 ignored tests. Formatting, Clippy with warnings denied, documentation,
three checker fixtures and whitespace checks passed.


## 2026-09-14 — Add checked destination reservation restoration

Added exact reservation operations to the restore provider/service/client. The
host configures a per-operation byte cap; its default is disabled. Checked
operations reject invalid/excessive requests before provider calls and retain
the original handle grant through admission. Providers explicitly refuse missing
support. AFS+ requires block alignment, preserves written data and logical size,
and supports the final rounded block ending at 2^64.

Extended the independent and AFS+ restore job with reservations in a hole and
beyond EOF. Added refusal, budget, revocation, final-address and remount checks;
the existing zero-write fixture covers alignment refusal and the in-backend
probe covers reservation admission. Added exact old/new object-record crash
oracles for preallocation while retaining an earlier snapshot, checking both
checkpoint slots and captured bytes/allocation.
The targeted crash run enumerated 626 states: 622 old and 4 complete new states.

Review identified two remaining qualification requirements: the existing core
loads full extent layouts for preallocation, and later COW writes allocate new
data blocks. The API cap alone cannot establish constrained-memory behavior or
near-full consumption of reservations. Recorded these explicitly in the queue
and test plan; they are implementation work before a complete reservation
capacity promise, not a reason to downgrade the full-preservation contract.

Validation: full workspace/all-features suite passed 344 tests, with zero
failures and 10 ignored tests. Formatting, Clippy with warnings denied,
documentation checks, three checker fixtures and whitespace checks passed.


## 2026-09-14 — Delegate decisions while the owner is away

The owner authorized choosing recommended design options without further
questions until the full queue is handled. Decisions must retain their rationale
and alternatives; public communication and hardware-write restrictions continue
to apply. Completion must include correctness and platform-interest review and
older-system degraded operation under resource constraints. Added those gates
to the queue so host success cannot substitute for constrained/native evidence.


## 2026-09-14 — Enumerate captured allocation and decide preservation modes

The owner chose separate full-preservation and content-recovery modes after
reviewing preallocation's capacity guarantee and restoration costs. ADR-078
records preservation/refusal for the full mode and explicit reservation-loss
reporting for content recovery. The precise PAX profile and destination
reservation qualification remain work.

Added bounded snapshot allocation pages, exposing semantic byte ranges and
unwritten state, including rounded tails and reservations beyond EOF. The VFS
backup facade checks authority and page bounds; unsupported providers explicitly
refuse. The extent reader checks a predecessor at page boundaries. No physical
addresses or sharing layout enter the consumer interface, and no disk or C ABI
changes were made.

The core fixture enumerates 140 fragmented written/unwritten records plus a
reservation at a 1 TiB offset after live truncation and remount. Page sizes 1,
7 and 64 preserve captured coverage with at most 32 device reads per page and
zero writes/flushes. Direct, empty, rounded-tail, bad-limit, directory,
out-of-range and stale-handle cases are covered. A valid-CRC overlapping extent
fixture exercises the cross-page check. The same VFS consumer reconstructs bytes
from allocation coverage and ordinary reads on independent and AFS+ providers;
revocation and in-backend authority exclusion have regression coverage.

Review found that the final rounded allocation can end at 2^64. Changed byte
conversion to multiply offset and length separately, then added an actual
preallocation/snapshot/remount regression at that boundary. Stopped the first
full-suite run to fix this before validation; its interrupted result is not a pass.

For the next archive unit, an exploratory GNU sparse 1.0 fixture recovered
12,288 exact logical bytes, including gaps and a trailing hole, through Python
3.9.6 tarfile and bsdtar 3.5.3/libarchive 3.7.4. The layout followed the
[GNU tar sparse 1.0 specification](https://www.gnu.org/software/tar/manual/html_node/PAX-1.html),
checked on 2026-09-14. This temporary-fixture experiment is an encoding input,
not archive, reservation or metadata-preservation qualification.

Validation: the final workspace/all-features run passed 341 tests with zero
failures and 10 ignored tests. Formatting, Clippy with warnings denied,
documentation checks, all three checker fixtures and whitespace checks passed.


## 2026-09-14 — Enforce separate destination restore grants

Implemented the owner's ADR-077 choice in a filesystem-neutral checked restore
service. Backup and restore use separate opaque public grant types and share
private admission machinery. Every operation checks the original handle grant;
revocation drains admitted calls. Linking checks both grants and avoids taking
the same read lock twice. Handle limits are reserved before creation, shared by
clones, and released after the provider handle drops even following revocation.

The AFS+ provider owns a writable volume and a selected empty directory. It
creates fresh entries without importing existing outside objects. One consumer
runs on an independent provider and AFS+, restoring nested names, file bytes,
sparse gaps, hard links, protection and three timestamps. The AFS+ test remounts,
checks exact values and sparse allocation, preserves an outside sentinel and
an older snapshot, and runs the exhaustive checker. Denial, malformed names,
metadata refusal, capacity cleanup and concurrent revocation have regression
coverage. In-backend probes check held write/link permits; compile-fail cases
reject cross-role grants and consumer backend access.

The API and qualification docs define the source-only compatibility boundary,
create-only destination and partial-work semantics. Q11 explicitly owns merge,
overwrite, resume and completion publication. This library test does not qualify
native host authentication, IPC, security-container/attribute transport or a
complete PAX archive. Those remain in the queue.

Validation: full workspace/all-features suite passed with 337 tests, zero
failures and 10 ignored tests. Formatting, Clippy with warnings denied,
documentation checks, all three checker fixtures and whitespace checks passed.


## 2026-09-14 — Restore existing metadata through the common COW tail

Added protection mutation and exact restoration of protection plus creation,
modification and change timestamps. Other object fields remain destination-owned.
The existing file data-policy setter shares the same metadata transaction helper;
it also rejects invalid timestamps before publication. Invalid caller metadata
has a distinct core error mapped to the existing VFS invalid-argument category.
No disk encoding, C ABI, feature identity or protection interpretation changed.

The timestamp audit found object and intent-log encoders accepting nanoseconds
that their readers reject. Shared validation closes that mismatch. Mutation
entry points validate supplied times before staging work, with a zero-write
namespace/log-window regression and encoder tests across all log operation kinds.
Valid encoded records retain their existing representation.

Tests cover files, directories and root, signed timestamp extremes, nanosecond
bounds, sparse shared storage, hard links, policy flags, no-ops, hidden targets,
read-only modes, open windows and uncertain publication. The crash oracle checked
346 states (342 old tuples, four new), preserving file bytes and snapshot metadata
with both-slot full checks. The sparse/shared fixture issued 28 KiB without
snapshots and 32 KiB with snapshots, four versus five metadata blocks and two
flushes in each case; neither wrote file data. These are fixture costs, not a
shipping resource profile. The AFS+ backup fixture changes live protection bits
before verifying the unchanged captured metadata and bytes.

The owner selected separate destination-scoped restore grants; ADR-077 records
that decision. The checked restore facade, native host authorization and full
PAX archive/restore preservation remain separate follow-up gates.

Validation: the final workspace all-features suite passed 326 tests, zero failed
and ten were ignored. Formatting, workspace Clippy with warnings denied,
documentation, all three checker fixtures and whitespace validation passed.
Portable C source already enforces the timestamp bound; no new native or
portable-C restore qualification is claimed.

## 2026-09-14 — Enforce revocable authority on the backup interface

Added the filesystem-neutral backup service and consumer facade under ADR-075.
The host owns grant issuance and privileged backend access. Every consumer call
checks its service-bound grant and retains a shared permit throughout backend
execution; revocation takes the exclusive lock and returns after admitted work.
Readers bind to their original grant. Final close releases the provider lease
before its explicit reader budget, even when authority has been revoked.

The same paged/streaming consumer runs against an independent provider and AFS+.
AFS+ captured names, metadata and bytes survive live write, deletion and remount;
old-service authority cannot access the new service. The independent provider
also changes live protection metadata. Tests distinguish denied calls from
backend access, preserve denied-read buffers, exercise duplication, busy deletion,
reader exhaustion, new grants and concurrent revocation. A compile-fail fixture
checks that the consumer facade has no unchecked backend hook.

Review corrected a missing-object error mapping: a captured directory entry
whose object is absent reports corruption. A valid-CRC damaged-image fixture
distinguishes that broken reference from an ordinary absent-object lookup,
checks zero source writes/flushes and requires independent checker rejection.

The Rust interface is additive and does not advertise C/IPC or OS support.
AFS+ protection mutation, exact restoration, sparse/attribute/security transport
and real host authentication remain owned follow-up work. A test collector is
not a complete archive/restore consumer.

Archive research on 2026-09-14 compared PAX with preservation metadata against
a dedicated container; the owner selected PAX and ADR-076 records the
direction, complete-versus-partial distinction and preservation gates. The
[GNU tar manual](https://www.gnu.org/software/tar/manual/tar.html) documents
PAX-based xattr/ACL storage and GNU sparse extensions. The
[tar 0.4.46 builder](https://docs.rs/tar/0.4.46/tar/struct.Builder.html) exposes
streamed entry and PAX-extension writing; host-path convenience methods do not
replace snapshot enumeration. Format choice still requires exact preservation,
unsupported-metadata refusal and independent extraction tests.

Validation: the final workspace all-features suite passed 319 tests with zero
failures and ten ignored tests. Formatting, workspace Clippy with warnings
denied, documentation checks, three checker fixtures and whitespace validation
passed. These are host-side results, not native authentication or full restore
qualification.

## 2026-09-14 — Configure snapshot limits before writable mount recovery

Added an explicit `mount_with_snapshot_limits` core entry point. It validates
runtime budgets and existing view admission before recovery can write, while
baseline mount APIs retain their feature mask. Snapshot orchestration fixtures
now use the public mount path, including their both-slot crash and churn checks.
No disk encoding or filesystem API v2 ABI changed.

The owner selected revocable host backup authority after comparing live-file
permission checks. ADR-075 records per-operation checks, historical access
independent of later live permissions, cleanup after revocation and the host
admission/conformance gates. Ordinary-user historical browsing remains open.

New mount fixtures cover all four modes, rejected budgets with pending durable
replay, unknown feature bits and corrupt selected roots with write/flush traps.
The one-to-90-view fixture measured nine versus ten mount reads before readers
or exhaustive checking, then verified every captured view. Shipping resource
profiles and host authorization remain independent requirements.
Snapshot-aware recovery preserved acknowledged live bytes and captured bytes
across 346 modeled interrupted states, with both-slot ownership checks after
recovery. The model covers full-write subsets and representative tears.

Validation: 311 workspace all-features tests passed, zero failed and ten were
ignored. Formatting, workspace Clippy with warnings denied, documentation,
three checker fixtures and whitespace validation passed. These are host-side
results; adapter authorization and native qualification remain separate gates.

## 2026-09-14 — Protect both checkpoint generations during reclamation

Implemented ADR-074's R <= P promotion boundary in all reclaim tiers, where P
is the oldest structurally valid checkpoint generation. Protected heads stop
consumption without moving their cursor. COW eligibility also consults the
previous snapshot registry; missing roots or unreadable older registry state
disable the optional optimization. Full-COW requests avoid that optional lookup. Runtime statistics report checkpoint blocking.

The post-deletion data-write oracle covered 352 modeled states: 347 retained
a registered view in a valid slot and preserved its exact bytes; five replaced
that registration. Both-slot full ownership checks accompany snapshot remounts,
create/delete cuts and churn. On the 512-block, 160-cycle fixture, minimum free
space was 472 blocks versus 475 under the former boundary; maximum ledger-retired
count was ten and final release transferred five protected blocks. TxAllocator
inline size on aarch64 rose from 936 to 960 bytes; heap state is additional.

Reclaim qualification exposed older tests' one-generation assumptions. The
fixtures require a protected maintenance generation before reuse and a two-root
steady state. Existing low-space recovery passed. The tiny-volume admission
oracle preserves its actual committed prefix rather than promising the same
capacity after adding retention. The original 64-block accounting workload
completes with two explicit maintenance publications: four additional flushes
and 64 KiB issued by those two steps, with final allocator bitmap residency of
four bytes. These are fixture costs, not a shipping resource profile. Previous-checkpoint preservation does not
change the ordinary non-snapshot opt-in in-place byte-failure contract.

Validation also exposed a documentation-checker race with Cargo deleting target
subdirectories. Discovery prunes excluded trees before traversal and propagates
errors in included source directories; temporary fixtures cover both behaviors
and deterministic output. The command-line interface and read-only default are
unchanged.

Validation: the full workspace all-features suite passed 306 tests with zero
failures and ten ignored tests. Workspace formatting, Clippy with warnings
denied, documentation checks, all three documentation-checker fixtures and
whitespace validation passed. These are host-side results; native hardware
and portable-C snapshot qualification remain separate gates.

## 2026-09-14 — Verify snapshot ownership and select stronger recovery retention

Added exhaustive registry/ledger and captured-namespace validation to the core
checker and enabled read-only snapshot inspection in afsplus-check. New-image
formatting has an explicit snapshot option; writable mount negotiation stays
closed. The checker reconciles lifetimes, namespace/housekeeping/quarantine,
metadata birth headers and bitmap accounting. Historical sharing does not use
the live reference tree. Physical blocks shared between views count once.

The full workspace all-features gate passed: 302 tests, zero failures and ten
explicitly ignored qualification tests. Formatting, all-target/all-feature
Clippy with warnings denied, documentation and whitespace checks passed.

The targeted gate passed ten core tests and one checker-facing test, including
resealed corruption cases and ownership sweeps across the existing create/delete
cuts and 160 churn cycles. A real checker gap was fixed: a disconnected directory
cycle could satisfy all incoming-link counts. Both live and historical graph
validation require reachability from namespace roots.

A post-deletion write experiment also clarified ADR-036: older shadow registry
storage could be reclaimed before its checkpoint slot was replaced. The newest
selected state stayed protected, so an additional COW check alone was not a
complete remedy. The owner explicitly selected stronger previous-checkpoint
protection. ADR-074 records the generation boundary, snapshot COW obligation,
pre-release compatibility limits and required low-space/crash qualification.
The handoff prioritizes that implementation before writable snapshot exposure.


## 2026-09-14 — Integrate persistent snapshot lifecycle into Volume

Implemented registry creation/deletion, mount-scoped reader leases, captured
namespace reads and resumable directory cursors under ADR-071/072/073. Creation
closes the intent window; registered snapshots override in-place writes. The
common transaction tail seals lifetimes, publishes both roots and transfers
eligible storage into ordinary quarantine. Deletion uses emergency headroom and
refuses active handles. Reclaim reports scan and promotion separately.

The full workspace all-features gate passed: 299 tests, zero failures and ten
explicitly ignored qualification tests. Formatting, all-target/all-feature
Clippy with warnings denied, documentation and whitespace checks passed.

Eight targeted memory-backend tests passed. Create cuts covered 197 modeled
states (193 absent, 4 present); delete cuts covered 115 (4 absent, 111 present),
checking membership and exact bytes. A 512-block fixture held an old view through
160 churn cycles with minimum free space 475 blocks and maximum retired count
12; releasing the view released its five protected blocks. Additional tests
covered private/shared COW, log-window capture and replay, cursor/handle remount
identity, near-full admission/deletion, uncertain final flush and exhausted IDs
without publishing a pending window.

Formatting and mounting snapshot images remain test-only. Ordinary mounts reject
the feature until full ownership checking and supported configuration are
qualified. These tests do not prove host authorization, backup restoration,
portable-C parity, shipping budgets or native durability. Q5 explicitly owns
snapshot management and historical-read access/revocation policy; the handoff
moves to checker integration and the real consumer.


## 2026-09-14 - Bind lifetime accounting to allocator transactions

Added namespace capture, housekeeping and lifetime-seal phases to the transaction
allocator. Births exclude released and log-sacrificed allocations. Last-live
retirements stay in the ledger; eligible transfers enter the ordinary queue only
with a successfully prepared lifetime mutation. Failed sealing poisons the
transaction, unsealed finalization fails, and caller mutations after sealing fail.
The finalized result carries replacement roots and writes into the common commit
publication path. Volume orchestration and feature activation remain later work.

The aarch64 inline allocator footprint measured 1,216 bytes with embedded
snapshot state and 936 bytes after making that state a lazy allocation.
Feature-absent transactions carry only the optional pointer. Enabled transactions
still allocate the accounting object; its heap and allocator overhead are
additional, so this is not a claim of lower total snapshot RAM or faster I/O.

Four tests use real bitmap, descriptor, allocation-root and reclaim state. They
verify housekeeping exclusion, exact replay births, guards and the delay between
ledger release and reallocation. The transfer crash matrix covers 115 modeled
states with exact ledger/quarantine ownership alternatives; deleting its metadata
barrier is a required failing control. The specification now states the sealing
and finalization obligations explicitly under ADR-071.

## 2026-09-14 - Prepare transactional lifetime edits

Added the lifetime mutation builder on the shared COW tree engine. It fetches
affected committed records and neighbors before editing, preserves birth across
splits, coalesces final equal lifetimes and adjusts retained totals with checked
arithmetic. Requested transfers are matched against their exact committed records
and the complete paged registry. Record and registry work budgets fail closed.
The result includes staged tree writes and runs requiring atomic quarantine.

Six tests compare disk-tree state with a per-block oracle, preserve old COW nodes,
exercise second-page snapshot protection and require invalid preparation to stop
before allocator calls. A 600-record sparse tree loads three lifetime records
for one retirement. This unit leaves the Volume commit hooks, real bitmap
quarantine, snapshot handles and integrated crash tests for the following work.

## 2026-09-14 - Read snapshot trees with bounded contextual checks

Added typed registry and lifetime readers on the shared key-page path. Registry
access validates control state and ID relationships. Lifetime pages check their
predecessor and lookahead so overlaps or missing coalescing cannot hide across
batch edges. Tests preserve sparse IDs, simulate cursor persistence/deletion
and wrap, inject semantic corruption and verify read counts with zero writes.

Added a constant-memory allocator-pool envelope for reserved-range exclusion.
Its output matches the existing enumerated placement across small geometries
and computes at the maximum region count without allocating a pool vector.
These are reader prerequisites; bitmap/log ownership and atomic lifetime edits
still belong to integrated snapshot transactions. No mount feature was enabled.

## 2026-09-14 - Bind snapshot roots in the checkpoint codec

Added ADR-073 before extending checkpoint encoding. Feature-absent checkpoints
retain their 96-byte payload; the snapshot extension appends two roots in a
112-byte payload. Selection rejects feature/payload mismatches without falling
back to older namespace state. Snapshot mounts stay unsupported through this
codec unit.

Independent full-block fixtures fix both encodings, with a separate bitwise
CRC32C construction. Tests cover lengths, root bounds, valid-CRC mismatches,
legacy byte stability and short encoder buffers. The short-buffer test also
removed an existing size-subtraction panic in checkpoint encoding. Updated the
C refusal fixture to carry extended checkpoint headers as well as the feature.

Reconciled the disk-layout draft's stale open allocation-architecture language
with the accepted ADR-067; exact wire freeze remains a separate gate.

Validation passed: 276 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation, whitespace and independent fixture reproduction.
The portable-C reader/writer, sanitizer and static-analysis gate passed; its
optional m68k and CMake checks were skipped for unavailable configured tools.


## 2026-09-14 - Add bounded key pages for persistent maintenance cursors

Added a shared-tree key-page reader for the snapshot ledger's durable cursor.
It seeks an inclusive key, stops at the caller's record budget and validates
visited subtree counts, generations, ranges and cycles. A physical key cursor
survives deletion of earlier records; wrapping explicitly revisits insertions
behind the cursor.

Three tests use a three-level, 1,024-record fixture and independent sorted-key
oracles. Traced reads equal reported reads and obey the fixture's bounded-page
budget. A mutation between reads deletes earlier records and inserts a new
key behind the cursor; successor enumeration and wrap preserve the expected
sets. This is a shared-core prerequisite; persistent cursor commit/recovery
belongs to the upcoming snapshot integration.

Validation passed: 274 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation and whitespace checks.


## 2026-09-13 - Add persistent snapshot record codecs

Added ADR-072 and the snapshot-record specification before codec implementation.
Registry and lifetime values use fixed-size independent byte fields, separate
AFST kinds and a negotiated incompatible identity. Validation covers reserved
bytes, contextual generation/range bounds, ID exhaustion and lifetime endpoints.
Conformance tests construct full checksummed leaf images and test corruption.

The core supported-feature mask deliberately excludes snapshot ownership until
integration. A named Rust mount/checker regression and an independent portable-C
probe require feature rejection. Checkpoint root binding, lifetime maintenance,
reader handles and crash-safe publication are the next integration work.

Validation passed: 271 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation and whitespace checks. The portable-C gate
passed its reader/writer, sanitizer and static-analysis checks. Its optional
m68k compiler and CMake checks were skipped because the configured tools were
unavailable; those runs provide no m68k compilation or CMake-package evidence.


## 2026-09-13 - Accept the integrated lifetime experiment and handle rule

John approved the lifetime-ledger prototype and selected busy-on-active-reader
snapshot deletion. ADR-071 records both decisions and amends ADR-070. The
proposal, Q4 and handoff link that decision. The follow-up wire record and
integrated resource/crash evidence precede acceptance of persistent support.
Documentation and whitespace checks passed.


## 2026-09-13 - Add checkpoint extraction with explicit loss reporting

Implemented `afsplus-extract` for Q11. It opens the source with an OS read-only
descriptor, mounts with NO_CHANGES, and exports readable checkpoint files into
a new destination. Numeric filenames prevent source names from becoming host
paths. A streaming JSON Lines manifest preserves original name bytes, object
and link identities, decoded core metadata and explicit loss findings.

The corruption fixtures recover healthy siblings beside damaged object records
and directory trees, refuse unreadable mount roots, and compare source bytes
before and after. Tests cover exact multichunk/sparse logical bytes, hard links,
Unicode names, timestamp precision, budgets and unknown-feature refusal. A
read-only spy panics on any source write or flush attempt; injected data-read
failure preserves the exact 64 KiB prefix, and pending durable log work is
reported as excluded from checkpoint extraction.

The tool requires a stable offline image. It reports unchecked payload integrity
and preserves partial artifacts on failure. Raw discovery through damaged roots,
full metadata-preserving restoration and transactional repair retain separate
Q11 gates. Corrected the milestone's stale assignment of repair to the grow-
resize milestone; recovery work follows Q11 under M05/M14.

Validation passed: 266 workspace tests, 10 ignored; formatting, Clippy with
warnings denied, documentation checks and whitespace checks.


## 2026-09-13 - Prepare the integrated snapshot accounting decision

Made the Q4 integrated candidate reviewable: physical-run lifetimes retain
birth across reflinks, a durable bounded scan transfers eligible runs to
ordinary checkpoint quarantine, and housekeeping metadata is excluded from
recursive snapshot ownership. The proposal spells out reader roots, ID
exhaustion, incompatible-feature negotiation, metadata admission and busy-on-
active-handle deletion. These are proposed experiment choices awaiting owner
discussion; no disk fields, feature bits or accepted ADRs were changed.

The next independent queue work is read-only extraction with an explicit loss
report and a corruption corpus. Documentation and whitespace checks passed.


## 2026-09-13 - Measure snapshot retention and reclaim traversal

Added five isolated Rust model tests for Q4. With one live block and one
snapshot-owned old block, oldest-generation FIFO retention exhausted 64-block
capacity after 62 unrelated temporary-file cycles (63 retired blocks, zero
free). At 256 blocks it exhausted after 254 cycles (255 retired, zero free).
Lifetime intersection with a rotating scan completed 256 and 1,024 cycles
respectively, retaining one block and leaving 62 and 254 free. It examined
two entries per cycle in this workload; each reclaim call is capped at eight
entries. After releasing all views, both policies reclaimed every queued
entry within the asserted budget.

The tests also cover birth/retirement equality, three release orders, progress
past 32 protected entries, failed allocation preserving live state, and a
negative control that detects deliberately reused snapshot media. These
results reject conservative FIFO retention for the model's bounded unrelated
churn target. They justify integrating more precise accounting for measurement.
The rotating in-memory scan is an experimental mechanism: sealed production
queue traversal, lifetime storage, reflinks, snapshot metadata, selectable
checkpoint protection, persistence and crash recovery need their own design
and evidence. No production format or filesystem API was changed.

Updated Q4, the proposal, qualification plan and handoff to include the
protected-head traversal requirement. Shipping budgets and the accounting
format remain explicit decisions. The complete audit queue is preserved.

Validation: workspace tests passed (261 passed, 10 ignored); formatting,
Clippy with warnings denied, documentation checks and whitespace checks passed.
The five model tests are included in that total.


## 2026-09-13 — Prepare the complete implementation handoff

Added the [audit work queue](implementation/audit-work-queue.md) so snapshot
work cannot displace salvage, repair, backup/restore, adapter/cache/device,
catalog/query/stream, security, tiny-file and sustained application findings.
Each row names its owner, existing evidence and completion gate.

John authorized the recommended next step: measure the conservative
oldest-snapshot retention barrier first. Q4 and the proposal record the
experiment order; the shipping accounting mechanism, registry encoding and
retention limits are not selected by that authorization. The handoff identifies
the relevant source paths, invariant boundaries and format-review requirements.
No snapshot implementation or additional runtime qualification is claimed.


## 2026-09-13 — Exercise repeated pressure and define the snapshot experiment

After the first push, added a 24-cycle near-full test with shared survivors,
forced growth failure, cross-block COW writes, bounded reclamation and remount.
Each cycle preserves exact bytes, checks allocation ownership and restores
capacity for the next cycle. This is live-sharing evidence, not snapshot
implementation evidence.

The [snapshot proposal](proposals/persistent-snapshot-prototype.md) separates
registry publication, retention accounting, in-place isolation and admission.
Source inspection identifies the relevant gaps: reclaim promotion has no
snapshot predicate, retirement records have no birth information, in-place
eligibility ignores snapshot ownership, and the fixed allocation pool cannot
pin arbitrary old checkpoint roots. Q4 links the proposal and keeps the
conservative-barrier versus precise-accounting experiment explicit.

Clarified the failure invariant against accepted ADR-062 and its existing
regression: opted-in in-place I/O can change user bytes before metadata
publication fails. Full-COW byte preservation and rejection before I/O are
separate stronger cases.

Validation: 256 Rust workspace tests passed with 10 explicitly ignored;
formatting, Clippy with warnings denied, documentation and whitespace checks
passed. The targeted repeated-pressure test passed. Persistent snapshot wire
semantics and implementation remain future work under the accepted direction.


## 2026-09-13 — Reconcile invariants and block writes after uncertain publication

Consolidated retention/reclamation, failure boundaries, catalog completeness,
change discovery and adapter/cache obligations in [spec/invariants.md](spec/invariants.md).
Corrected [recovery](docs/19-recovery-and-maintenance.md): catalog rebuild is
possible; discarded stream history requires reset and rescan. Clarified that
reserved and quarantined blocks are legitimate allocated ownership classes,
and that an acknowledged fsync prefix cannot be discarded during recovery.
These consolidate existing contracts without new wire fields or API signatures.

The [coverage map](testing/book-review-qualification.md#normative-coverage-map)
links every family to executable tests or an owner experiment. New failure
coverage reproduced a correctness defect: after a complete checkpoint write
and failed final flush, the mounted writer could mutate again from stale roots
and allocation state. The common commit tail now sets its remount-required
state before attempting checkpoint I/O and clears it only after successful
adoption. Completed writes reporting errors and failed post-publication reads
receive the same protection. Existing WindowPoisoned adapter mappings are
reused; pre-publication transient failures keep their safe retry behavior.

Added three tests: uncertain final-barrier mutation rejection; completed-write
and adoption-read error handling; and a dynamically enumerated replacement
write/flush fault matrix with exact namespace, content and checker oracles.
The targeted six-test fault suite passed, including existing retry controls.

John selected consistent filesystem snapshots first. [ADR-069](adr/ADR-069-consistent-snapshots-first.md)
records the direction, the required snapshot protection against in-place
updates and a backup/scanner prototype. Q4 retains the bounded-retention and
persistence questions. New Q11 and Q12 own salvage/restore and storage-stack
qualification; the roadmap places their experiments before safe daily storage
claims. Snapshot storage and API encoding were not implemented or frozen.



John subsequently selected persistent snapshots first; [ADR-070](adr/ADR-070-persistent-snapshot-priority.md)
records that choice. The allocation-root pool has capacity for two selectable
checkpoints plus one commit, so arbitrary snapshot pins cannot reuse that
capacity proof. The persistent registry and retained-ownership design need
explicit prototype evidence before wire/API decisions.

Full validation: 255 Rust workspace tests passed, 10 explicitly ignored;
formatting, Clippy with warnings denied, documentation and whitespace checks
passed. The parent roadmap checker and all 13 checker fixtures also passed.
No native or long-duration qualification is implied.

## 2026-09-13 — Complete BFS book review and fragmented allocation regression

Read the twelve chapters and construction-kit appendix of Giampaolo's
*Practical File System Design* against source revision
`44ea5ebb7ba1efb5fd37588956700500f1047e1e`. The
[review](docs/33-practical-filesystem-design-review.md) records source identity,
printed pages, chapter dispositions and corrections. Earlier research had
misattributed substring-search costs to OR and overgeneralized alignment,
old duplicate performance and the absence of COW antecedents.

The allocator reproduced the book's repeated large-request fallback problem.
For 64 isolated free blocks in the 512-block allocator fixture, the original
loop made 385 allocation searches and examined 169,536 bitmap positions.
Keeping a reduced request cap for one invocation cuts those counts to 70 and
8,256 respectively (81.8% and 95.1% reductions). This is a work-count
measurement, not a throughput claim. The cap uses the existing stack variable;
two u64 counters add 16 bytes to AllocStats. No on-disk encoding or barrier
changes. The regression checks exact extent positions and verifies a later
request can use a newly freed contiguous run. Residual within-region rescans
remain measurable follow-up work.

Added a second independent byte-vector oracle covering three seeds and 576
mixed write/truncate/preallocate operations, 36 exhaustive-check/remount
boundaries, short-read buffer preservation and reflink isolation. It passed.
Existing tests already cover interrupted replay, batch failure rollback,
near-full progress and open-unlinked cleanup, so those engines were retained.

Refined catalog completeness and stream rescan requirements with explicit
backfill, enumeration/cursor handoff, reset identity and notification overflow
gates. Cache coherence, VM reentrancy, query predicates and metadata-preserving
transport have owner experiments in the
[qualification plan](testing/book-review-qualification.md). These are
requirements for their respective future facilities, not claims that those
facilities were implemented. Accepted ADRs and unresolved format choices were
preserved; allocation tuning is allowed by ADR-067's runtime-policy decision.

Validation on this macOS arm64 host with an isolated temporary Rust 1.98.1
installation: `cargo test --workspace --all-features` passed 252 tests with
10 explicitly ignored tests; `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets --all-features -- -D warnings` passed.
`make toc`, `make check-docs` and whitespace validation passed. The ignored
qualification workloads, long soak and native hardware gates were not run.
All changes remain local; no mount, hardware write or publication was made.


## 2026-09-03 — Portable C allocates and logs its first COW data block

The independent ABI-1 writer crossed the allocation boundary with
`afspw_write_file_block_cow`. The API accepts one complete caller-assembled
logical block, rejects shrinking ranges and nonzero data beyond a partial EOF,
and keeps the 8 KiB no-heap contract. Its allocator decodes the fixed-key
allocation root, selected region descriptor and bitmap pages independently of
Rust, recross-checks all free counters, proves every existing log extent is
base-checkpoint-free and skips those reservations before choosing a block.
The same 8-to-64-block runtime emergency floor as the Rust writer protects
recovery capacity.

Durability is two-stage: write/flush the new data, then write/flush the
version-3 record. Distinct statuses and stages identify data-write,
data-flush, record-write and record-flush uncertainty. Torn data and torn
record retries both recover through a fresh probe; Rust replay observes the
exact 123-byte final-block replacement and the checker accepts the resulting
allocation. The cached seven-record path needs 25 reads, the 8 KiB path 223,
and either torn retry 50. A 512-block fixture reserves every ordinary-growth
block in a prior record while retaining the 16-block emergency floor, and
proves ENOSPC with zero writes; injected descriptor and bitmap failures report
exact LBAs. Strict C, sanitizers, static analysis, CMake and m68k compilation
retain a 768-byte maximum writer frame.

## 2026-09-03 — Portable C emits its first version-3 data mutation

The ABI-1 C writer now appends data-free truncates of existing regular files.
It supports sparse growth, zeroing and block-aligned shrink without allocating
media or publishing a checkpoint. A same-size request succeeds with an
explicit zero-I/O result. Unaligned shrink instead returns
`AFSPW_ERR_TAIL_REWRITE_REQUIRED`: exposing stale bytes from a materialized
partial tail would be worse than delaying that case until the COW data
allocator exists.

The C reader observes the new size immediately, and Rust replay verifies both
zero shrink and growth to 12,295 bytes with an exact zero-filled suffix before
running the checker. Version-3 feature absence and unsupported tail rewrite
both write nothing. Torn-record retry and flush uncertainty retain the same
diagnostic contract as namespace records. At a seven-record prefix the cached
path uses 21 reads, the 8 KiB path uses 178, and a cached retry uses 42; the
shared writer setup refactor lowers the largest measured m68k writer-function
frame to 740 bytes.

## 2026-09-03 — Portable C creates empty files without an allocator

The ABI-1 portable writer now emits an empty regular-file create into one
preallocated intent slot. Its semantic preflight validates the complete
namespace prefix, proves the parent directory and name absence, advances over
every prior logged create and returns the exact monotone object ID. The
operation allocates no data block and publishes no checkpoint; Rust replay
materializes its metadata later.

The C reader sees object 19 and zero durable bytes immediately after the
write, then Rust replay preserves the same identity and the checker accepts
the result. A case-insensitive collision and missing parent return the exact
create-lookup failure before any write. A separate valid checkpoint with
`next_object_id = UINT64_MAX` proves exhaustion is distinct from corruption
and also writes nothing. A second valid-checksum checkpoint regresses the
watermark below the highest committed object; a bounded right-edge object-map
lookup catches it before the writer can reuse an identity. The successful
paths retain 144-read 8 KiB and 21-read cached ceilings, one block write, one
flush and the bounded m68k stack contract.

## 2026-09-03 — Portable C trades caller RAM for 7x fewer writer reads

The bounded C writer used only 8 KiB, but its deliberately stateless semantic
preflight reread the same log and metadata blocks many times. On the fixed
seven-record qualification image that cost 142–152 logical reads before one
log write. A call-local LRU now uses only complete extra blocks supplied after
the mandatory workspace. It owns no memory, retains no media pointer and is
discarded on every return; the exclusive-writer and uncertain-I/O contracts
are unchanged.

The 8 KiB path remains a first-class, cross-replayed low-memory configuration.
The recommended size is 56 KiB: the original two-block workspace plus twelve
cache entries. That profile needs 21 reads for rename, delete or replacement,
and 42 across a torn-write retry's two full preflights, while preserving the
single write and flush. The executable gate fixes all three numbers as upper
bounds, compiles the same implementation for m68k and validates both memory
profiles through Rust recovery and the checker. Its main m68k writer frame is
760 bytes in the current toolchain and carries a 1 KiB regression ceiling.
This is a memory/performance choice for adapters, not a second disk format or
ABI. A transient first-log
read failure additionally proves exact stage/LBA reporting, zero media writes
and a clean fresh retry; the probe can print every preflight LBA under an
environment switch for adapter diagnostics.

## 2026-09-03 — Intent replay makes final namespace removal bounded

Durable-log replay no longer retires the complete layout of a final delete or
replacement victim. It preclaims every logged data run before allocating
metadata, then moves committed final victims into ADR-066 orphan state in the
same replay checkpoint. If object 2 was never used, its record, empty tree root
and first entries are created inside that transaction-local COW overlay; there
is no preparatory checkpoint that could stale an unapplied log. A logged write
or truncate followed by final removal publishes the final data layout under the
orphan identity, while non-final hard-link removal still decrements normally.

The focused gate replays a 33-extent victim with ordinary free space nearly
exhausted and proves the replay retires fewer blocks than the victim has
extents, writes no data and preserves every byte. Four durable records create
64 orphans in one checkpoint, and every modeled write/flush/torn cut of a
replacing replay converges to the new visible source plus the byte-exact old
target in orphan state. Shared data remains referenced until bounded cleanup.

With that invariant established, the ABI-1 C writer now exposes regular-file
delete and replacing rename alongside non-replacing rename. Each operation
performs a fresh semantic preflight, writes one preallocated log block and
flushes once. The cross-language gate observes the namespace in C, replays it
in Rust, verifies one or two expected persistent orphans and runs the exhaustive
checker. The seven-record fixture measured 142 reads for delete, 148 for
replacement and 152 for non-replacing rename, all with 8 KiB caller scratch;
the call-local cache described above subsequently addressed that
classic-hardware optimization target without removing the 8 KiB path.

## 2026-09-03 — Portable C performs its first durable mutation

The independent C99 path now appends a non-replacing regular-file rename to
the preallocated intent log. It freshly probes and validates the complete
checkpoint-plus-log namespace, proves the source type and absent destination,
writes one record and issues one flush. Rust replays the C-produced sequence-8
record and the exhaustive checker validates the materialized image. A torn
record leaves the prior seven-record namespace, can be overwritten in the
same slot after a fresh scan, and never publishes a hybrid rename. Write and
flush callback failures have separate explicitly uncertain results; full-log
and existing-target cases perform zero media writes.

The deliberately narrow initial surface exposed a correctness trap during its
audit: replay could retire a delete or replacement victim directly. The next
lot above closed that gap and expanded the same ABI. The original
maximum-prefix measurement remains 152 uncached logical-block reads, one write
and one flush with 8 KiB caller scratch.

## 2026-09-03 — Q3 allocation architecture accepted

The owner accepted ADR-067 after the post-orphan low-space requalification.
Epoch 1 therefore keeps triple-version bitmap pages and region descriptors,
the deterministic fixed-topology `3N` allocation-root pool, authoritative
bitmaps and the segmented reclaim queue. Runtime emergency headroom, rover,
locality hints and maintenance batch sizes remain tunable policy rather than
new wire fields.

This closes the allocation architecture question, not the global format
freeze. M14 still owns exact byte-layout review, overflow bounds, feature
negotiation and independent cross-reading before the wire epoch can be called
stable.

## 2026-09-03 — Emergency headroom makes ENOSPC recoverable

The post-ADR-066 Q3 pass added a soft transaction-allocation floor rather
than another reserved disk area. On volumes of at least 64 blocks the runtime
keeps `clamp(ceil(total blocks / 32), 8, 64)` raw blocks for destructive and
recovery work. Normal user/data and metadata allocations are checked against
the same floor, including allocations performed only while sealing the
reclaim queue, so a rejected growth transaction still issues no writes.
`statfs`, `afsplus-info` and `afsplus-dump` now distinguish raw free,
emergency headroom and normally available blocks.

The filesystem-facing VFS uses the hidden orphan transition for every final
file unlink or replacement, not only while a handle is open. Visible
namespace removal is therefore independent of the target's extent count;
cleanup is idle, bounded and restartable. A 160-region test grows the user
namespace past a single tree leaf, fills a highly fragmented file down to the
emergency boundary, removes its name in two bounded checkpoints, then drains
it eight extents at a time. It deliberately configures the general reclaim
budget to one block, which exposed a negative-progress loop; orphan cleanup
now promotes at least 16 blocks per step and the same test converges.

The optimized maximum-region run kept 64 emergency blocks and finished with
85 raw/21 normally available blocks after reserving 262,011 data blocks. It
issued 31 reads, 16 writes, 65,536 written bytes and two barriers, dirtied all
nine bitmap pages, one descriptor and one allocation-root node, and retained
the 32 KiB allocator-RAM bound. The 512-block and multi-node sweeps refused
fills that would cross their 16- and 64-block floors; every accepted fill
still completed destructive progress. These are memory-backend qualification
figures, not device-latency claims.

## 2026-09-03 — Open-unlinked files gain a bounded crash-restartable lifetime

The owner accepted ADR-066 after the first Q3 low-space measurements showed
that final unlink could not safely retire an arbitrarily fragmented file in
one reserve-sized transaction. The implementation reserves object ID 2 as a
lazily-created hidden directory. A final visible link with live handles moves
there atomically, including an open atomic-replace target, while the file's
ordinary link count stays one. Existing handles retain read, write, truncate,
data-policy and fsync identity; public lookup, stat, new open, hard-link and
rename cannot rediscover internal state.

Cleanup uses extent-tree subtree counts to read only a bounded tail. Each
maintenance transaction removes at most the configured logical-extent budget
and publishes a smaller orphan before a final small transaction deletes the
entry and record. Read-write mount and filesystem sync advance one orphan;
adapters also have an idle hook and a pending count. The checker treats object
2 as an explicit feature-gated root and rejects malformed names/types,
visible aliases and a missing feature bit. The dump labels the internal role;
the independent C reader negotiates the `RO_COMPAT` bit but returns
`NOT_FOUND` through its ordinary API.

The qualification exercises multiple handles, hard links, name reuse, legacy
fallback, open-target replacement, resumable multi-extent cleanup and every
modeled write/flush cut of insertion, post-unlink update, cleanup and
replacement. Every recovered intermediate is required to be checker-clean and
semantically old or new; no mount-only oracle is used.

The recorded optimized memory-backend run removed exactly eight of 33 sparse
extents in 537 microseconds and left 25 for restart. It issued 24 reads, eight
writes, 98,304 read bytes, 32,768 written bytes and two barriers; the committed
transaction contained five metadata blocks, one bitmap page and 512 bytes of
allocator bitmap payload. These are qualification evidence for this build,
not a hardware-performance claim.

## 2026-09-03 — Q3 qualification exposes the missing emergency reserve

The first combined near-full allocation qualification found a liveness gap
before the prototype encoding could be proposed for freeze. A successful
preallocation could consume the last raw free blocks and leave the volume
checker-clean but unable to allocate the metadata needed by unlink. On the
512-block single-region fixture, unlink allocated four blocks after one
quarantine promotion and failed with two raw free blocks; on the 145-region
fixture it allocated five blocks, wrote two reclaim structures and failed
with three raw free blocks. The observed thresholds are measurements, not a
safe reserve definition.

The same suite pinned the good side of the boundary: a failed ENOSPC attempt
issued zero writes and zero barriers, a near-full unlink survived every
modeled power cut as exactly the old or new generation, and bounded reclaim
restored space. One maximum-size 1-GiB region reserved 262,075 blocks in 4.7
ms in the recorded optimized memory-backend run, with 31 reads, 16 writes,
two barriers, nine bitmap pages, one descriptor, one allocation-root
record/node and 32 KiB of
allocator bitmap payload. A 145-region transaction crossed the allocation
root leaf boundary while staying below 64 KiB.

This made the dependency on open-unlinked/orphan cleanup explicit. Unlinking
a highly fragmented file cannot reserve metadata proportional to the whole
file on a classic volume; namespace removal needs a bounded orphan step, then
restartable reclamation. Q3 therefore stays open while that M03 machinery and
the already-specified free-versus-available headroom contract are implemented
and requalified.

## 2026-09-03 — Checkpoint selection regains Rust/C parity

The checker corruption corpus made Rust reject three reserved checkpoint
fields, but the independent C reader still accepted them when the block carried
a matching CRC. That divergence let Rust and C select different retained
generations from the same forged or future-format image.

The portable decoder adopted the same zero requirements for common-header
flags, common-header owner and payload flags. Its Rust-produced fixture mutates
each field independently, reseals the block, and tests both safe fallback and
the two-invalid-slot failure. A focused Rust codec regression pins the other
side of the contract so either implementation changing alone becomes visible
at its normal developer gate.

## 2026-09-03 — Prototype commands become integration-grade tools

The formatter moved from a convenience binary to the official `mkafsplus`
contract. It gained named compatibility profiles, reproducible UUID/timestamp
inputs, stable JSON and diagnostics, and same-directory staging so a failed
format never exposes a partial destination. This also uncovered an old
geometry wart: arbitrary MiB sizes inherited the total block count as their
region size and therefore failed whenever that count was not a power of two.
The official formatter uses the fixed maximum region geometry and accepts
ordinary image sizes.

`afsplus-info` and `afsplus-dump` deliberately split cheap recognition from
forensic depth. Info reads only identification and both checkpoint slots;
dump walks the full committed object/directory/extent/allocation/reclaim/shared
state and the durable log prefix. Both use a real OS read-only descriptor plus
fail-closed mutation methods. Black-box tests run all three installed command
names, pin the schema and exit contracts, distinguish corrupt media from host
I/O, and repeat both inspectors against a populated mode-0444 image while
proving its bytes and permissions unchanged.

## 2026-09-03 — Rust codec failures gain stable case identities

Five dependency-free fuzz targets began exercising the identification,
checkpoint, typed-tree, object-record and intent-log codecs. Every seed must
round-trip to one canonical block; the deterministic engine mixes raw CRC
failures with resealed semantic mutations, short inputs, multi-byte changes
and bounded extensions. The default gate runs 20,480 cases and records the
target and case before each call.

Before publication, the same engine also completed an extended run of 100,000
cases per target (500,000 total) without a panic or canonical round-trip
failure.

The separate `.afrf` artifact format keeps the target, stable case number,
seed-schema version and exact bytes needed for replay. This made failures
portable across machines without requiring `cargo-fuzz` or a particular LLVM
runtime, while leaving room for those engines to call the same pure decoder
entry points later. Confirmed artifacts placed in `fuzz/regressions/` become
permanent repository-gate inputs.

## 2026-09-03 — Checker corruption becomes a replayable corpus

The Rust checker gained a twelve-image corpus spanning identification,
checkpoints, typed trees, object records, allocation bitmaps and the intent
log. Each surface has a raw integrity failure and a valid-checksum semantic
failure. Fixed UUIDs, timestamps and geometry make the sparse images and
their exact JSON reports byte-reproducible; a versioned manifest names every
changed LBA and byte range so another implementation can replay a failure
without reading the Rust tests.

Building the cases exposed two gaps before publication. Checkpoint decoding
accepted nonzero reserved header and payload fields, so both retained records
could carry unknown state without failing selection. Intent scanning also
treated a nonzero undecodable record exactly like a zero unused slot. The
decoder now rejects the checkpoint fields, while the log keeps its legal
crash-boundary semantics and adds a forensic warning that distinguishes torn
or damaged bytes from an empty tail.

## 2026-09-03 — Portable C overlays the durable namespace

The independent C99 reader gained the namespace half of the intent-log view.
It now resolves and enumerates the final checkpoint-plus-log directory state,
chases rename chains without recursion, honors replacement, and keeps object
identity correct across hard-link deletion and rename. The same semantic pass
checks source/target preconditions, monotone create IDs and write/truncate
expected sizes before file bytes are exposed. Calls retain the 8-KiB
caller-owned scratch contract; an explicit cursor makes ordered iteration and
small-buffer retries possible without hidden allocation or global state.

The Rust-produced fixture grew from three to seven durable records and now
combines delete, write, truncate, create, rename chains and replacement.
Portable C matches both surviving oracle files and the final
case-insensitive namespace, while four valid-checksum mutations demonstrate
that semantic inconsistencies fail at the responsible record. A seventh AFZF
seed carries a successful namespace lookup through the deterministic
ASan/UBSan mutation and replay gate. The reader also learned
[ADR-065](adr/ADR-065-persistent-data-update-policy.md)'s per-file policy flag
and rejects the flag without its volume feature, keeping the Rust and C format
boundaries congruent.

## 2026-09-03 — The per-file data policy becomes persistent

ADR-062 had fixed the hybrid data-update semantics but deliberately left the
per-file opt-in unencoded, so the choice evaporated at every mount. ADR-065
(approved on board decision #63) assigns the encoding: object flag bit 1,
`OBJECT_FLAG_DATA_IN_PLACE`, protected against silent loss by the validated
object-flags namespace every implementation already enforces, plus `COMPAT`
bit 0 `org.aros.afsplus:data-policy` activated by mkfs profile with
shared-extent-style congruence — a flagged record on a volume without the
feature is corruption in the read path and the checker. The core gained
`set_file_data_policy`/`file_data_policy` (a metadata-COW record update) and
the VFS a `DATA_POLICY` capability with handle-based set/get; the write path
takes the in-place route when the record carries the flag, under unchanged
ADR-062 eligibility. One real bug surfaced while wiring it: layout staging
rebuilt object flags from scratch, so any write dropped the freshly set
policy bit — the persistence regression caught it immediately, and the bit
now travels through every layout rewrite. Seven persistence regressions plus
two VFS API tests cover remount survival, in-place stats, clearing, feature
refusal, planted-flag fail-closed on both sides, shared-block COW fallback
and the power-cut contract through the flag.

## 2026-09-03 — Portable C reads the durable log view

The independent C99 implementation no longer stops at the selected
checkpoint when a writer has acknowledged later fsync groups. With an 8-KiB
caller-owned scratch buffer it scans the bounded intent-log prefix, validates
record shape and sequence, content-verifies every referenced replacement block
and reports the first excluded tail slot and LBA. No replay is written to the
device.

The first view layer resolves create, write and truncate records into final
file sizes and bounded reads. Its cross-language fixture leaves three durable
records over a Rust checkpoint, then C reconstructs an existing file after
write-plus-shrink and a file that exists only in the log, byte-for-byte against
Rust-produced oracle files. Damaged data, sequence gaps and missing feature
contracts have exact regression expectations. Rename and delete explicitly
disable the file-data view until namespace identity is overlaid rather than
returning plausible but incomplete bytes.

The same decoder is now the sixth compact mutation seed. Parent and child
tree item counts are also compared during descent, closing an independent
review observation where an internally consistent parent over-claim could
otherwise hide reachable entries. Strict C99, sanitizer, analyzer, CMake and
m68k compilation remain one gate so future integrators get the same failure
coordinates as the in-tree build.

## 2026-09-03 — Portable C failures become replayable artifacts

The first portable-reader expansion substantially increased the amount of
untrusted C parsing, so its next lot hardened that surface before adding more
features. A host recorder now executes successful operations against a real
Rust-built image and writes only the blocks that were actually observed. Five
12–48-KiB packets retain checkpoint probing, object lookup, both extremes of a
303-entry directory and a complete directory-to-file read without copying or
carrying the complete disk image through every mutation.

Each packet receives 4,096 stable mutations under AddressSanitizer and
UndefinedBehaviorSanitizer in the ordinary repository gate. Every operation
header byte and the first 128 bytes of each observed block are tested both
with a broken checksum and with a recomputed CRC, so structural checks are not
hidden behind the checksum gate. Block-wide deterministic flips, overwrites,
multi-byte changes and truncations follow. The progress record is updated
before execution, so a crash names a seed and case number; the same case can
export an exact `.afzf` artifact and the replay tool reports symbolic operation
results plus the terminal stage and LBA. This turns parser failures into small
regression inputs an integrator can keep and exchange.

The harness also exports the standard libFuzzer callback. The installed Apple
command-line tools do not currently contain its link runtime, so that engine
is opportunistic rather than a hidden local prerequisite; the deterministic
sanitizer runner is the portable mandatory gate. The remaining M01 corpus is
listed by wire surface instead of being represented as a generic “fuzzing
open” checkbox.

## 2026-09-03 — The independent portable C reader starts executing

The first ADR-029 implementation slice added a C99 reader that does not link or
generate code from the Rust codecs. Caller-owned block I/O and one 4-KiB
scratch block are sufficient to validate the identification record, negotiate
the implemented feature summary, validate bounded geometry and structurally
select between both retained checkpoints. The public reader surface is
versioned before expansion so future profiles do not depend on native C struct
serialization.

Its cross-implementation gate formats and advances an image with Rust, compiles
the reader under strict C99 warnings, and asks C to select it. The same binary
injects a bad newest checkpoint, valid-checksum structural corruption, two bad
checkpoints, an impossible duplicate generation, a selected-state feature/root
mismatch, a bad identification checksum and a short scratch buffer, with
structured stage/LBA/per-slot diagnostics. The feature/root case proves that
selection does not hide corruption beneath a newer structurally valid
checkpoint by falling back. Sanitizer, CMake-example and available AROS-m68k
compiler gates catch host-language and integration faults; Rust's checker
validates the untouched source image. Object-tree traversal and file reads form
the next reader slice rather than being simulated through the Rust
implementation.

The next slice made that bootstrap useful without turning it into a second
host runtime. A 303-file Rust fixture forces object-map and directory descent
through internal nodes; C enumerates the root by ordinal, resolves a file
object, and reproduces its bytes through bounded 777-byte reads. A wire-valid
extent leaf over the same Rust-written data adds sparse-hole and extent-floor
coverage. Tree CRC, valid-checksum identity, generation/owner/level/range/count
invariants, typed object/directory/extent values and exact failing LBAs are now
checked along every accessed path. The API remains additive: the original
entry shape stays intact, new operations advertise capability bits, and all
storage remains caller-owned.

Independent review also caught that the checkpoint payload's reserved `flags`
word had no explicit reject-or-ignore rule in either implementation. Rather
than make the C reader silently stricter than Rust, the freeze choice is now
[Q10](implementation/open-questions.md) and must be closed during M14.

## 2026-09-03 — FSKit durability moves to write replies

The owner chose a scoped workaround instead of debugging the macFUSE/Apple
FSKit boundary. The macOS FSKit mount treats each delivered data write or
truncate as an implicit durability point and replies only after the AFS+ log or
checkpoint is durable. This preserves host `fsync` correctness even when the
transport omits `FUSE_FSYNC`, without changing the core, on-disk format,
host-neutral FUSE behavior or AROS adapters and without requiring the legacy
kernel extension.

The tradeoff is intentionally visible: small host writes lose cross-request
batching and can each pay the data-and-record barriers. A protocol regression
crashes immediately after successful write and truncate replies without an
`fsync` request. The real macFUSE mount then passed its syscall-boundary oracle:
host `fsync`, immediate second-descriptor inspection, clean unmount and checker
all succeeded without administrator authorization. The host benchmark contract
must label the fallback.

## 2026-09-03 — Qualification order follows executable targets

The owner reordered the four platform-validation stages so an unavailable
bare-metal target cannot block executable qualification. The sequence is
Hosted MacAROS, Amiga 500/m68k emulation, the physical A500, then native
MacAROS on Apple Silicon after that system exists. These are three target
platforms: the emulator and physical A500 are two stages for the same classic
target.

This ordering change does not weaken the reopened host `fsync` verdict. The
real FSKit mount proves that the syscall returns without delivering a FUSE
`FSYNC` request to AFS+, while direct VFS and FUSE-protocol tests exercise the
expected durability path. The defect is therefore below the AFS+ engine, in
the macFUSE-FSKit/Apple-FSKit path. The trace does not by itself assign the bug
between macFUSE's shim and Apple's FSKit plumbing; a stock macFUSE loopback
reproducer is the attribution gate.

## 2026-09-03 — macFUSE FSKit reopens the host fsync gate

Re-running the real mount after wiring the VFS log exposed a false-positive in
the Mountable Alpha-0 host gate. On macOS 26.6.2 (25G83) with macFUSE 5.3.3's
FSKit backend, both Rust `File::sync_all` (`F_FULLFSYNC`) and a direct
`fsync(2)` returned after dirty data reached the FUSE `WRITE` callback but no
FUSE `FSYNC` callback was delivered. Instrumentation distinguished `WRITE`,
`RELEASE` and `FSYNC`; only `RELEASE` arrived when the descriptor closed. For
two seconds after the syscall, a second descriptor on the image saw the same
checkpoint generation and zero pending intent records. The later rename or
unmount checkpoint had allowed the old round-trip test to pass.

The ignored real-mount test now inspects the live backing image immediately
after host fsync and requires either a replayable record or a newer checkpoint.
It intentionally fails on that FSKit combination, while the direct FUSE
protocol, VFS, AROS and C-bridge fsync tests pass. M08 is therefore partial
again. A safe FSKit-specific write-through workaround would add barriers to
every delivered write, so it requires measurement and an explicit policy
choice rather than being enabled silently; the preferable fix is for the host
stack to deliver FUSE `FSYNC` before returning from the syscall.

## 2026-09-03 — Logged data fsync reaches the portable Rust adapters

The portable VFS now routes existing-file writes and truncates through the
version-3 data-update window when the volume advertises that incompatible
feature. Reads and stats see the pending logical layout before publication;
`fsync` writes the bounded data/record barriers without advancing a checkpoint,
while oversized/full groups, namespace operations, reflinks and filesystem
sync publish the window through the ordinary checkpoint engine. Log-free and
older volumes retain their immediate checkpoint behavior.

The FUSE protocol, packet-neutral AROS Rust adapter and current C ABI bridge
inherit this path from the VFS. A trace test proves a successful VFS fsync
writes no checkpoint slot and that remount recovers its bytes; adapter tests
cover pre-fsync visibility, remount and checker cleanliness. The next
portability boundary is the independent portable-C filesystem implementation,
followed by real-device barrier measurements.

## 2026-09-03 — Intent-log writes and truncates survive nested recovery crashes

The accepted ADR-063 architecture now covers committed-file data changes in
the Rust core. Experimental record version 3 names freshly allocated COW
replacement blocks for writes and, when needed, one zero-tailed block for a
partial shrinking truncate. Data is flushed before the record; replay checks
the complete-block CRC, claims those exact blocks while they are FREE in the
base checkpoint and publishes the final layout through the ordinary COW
transaction engine. Multiple fsync groups on one file collapse into one final
metadata mutation without weakening their monotone-prefix recovery contract.

The new matrix covers old-or-new cuts around the live write, every write and
flush of recovery itself followed by another remount, write-plus-rename,
sparse growth, aligned and partial shrink, successive writes, and a reflink
split whose other owner retains the old bytes. Release workloads measured
2.200 writes/2.032 flushes per 200-byte logged append and 2.134/2.032 per 4 KiB
DB hot-set rewrite, versus 9.037/3.000 for checkpointed append. These are
memory-backend structural counts, not Apple or Amiga hardware results.

Review exposed a compatibility trap: an older version-2 scanner treats an
unknown record version as a torn tail. ADR-064 therefore assigns
`org.aros.afsplus:intent-log-data-updates` as a dependent INCOMPAT identity so
such readers reject the volume before silently losing an acknowledged fsync.
The numeric wire remains experimental; VFS/C parity, real-device flush testing
and the M14 format review remain open.

## 2026-09-02 — Q1 and Q2 architecture blockers closed

The shared-extent implementation made the private/shared write fork executable,
so Q1 was settled by measurement rather than preference. A runtime-only
private-in-place prototype was compared against full data COW on random 4 KiB,
database-hotset, append and reflink-first-write workloads. The large random
case fell from 26,972 to 8,000 device writes, 24,870 to 3,000 allocations,
21,936 to 2,000 retirements and 1,767 to one final extent. The database hot
set retained a smaller but material advantage; append and shared first-write
rows were identical because they correctly remained COW. Power-cut and
injected-error tests demonstrated the price directly: an old metadata
generation may expose old, new or torn overwritten bytes, and a returned error
does not imply byte rollback. ADR-062 therefore selected full COW by default
plus a persistent, explicit per-file private-in-place opt-in, never automatic
detection.

Q2's optimized re-run held the earlier result: a logged Git-style ref update
cost 2.152 writes and 1.032 flushes per operation, versus 10/3 for one
group-commit checkpoint and 26.992/6.998 for the original sequence. Review of
the executable record format caught an overclaim in the old report: version 2
can replay create/delete/rename and created-file content, but not writes or
truncates of existing files. ADR-063 accepted checkpoint COW, bounded group
commit and the intent log as the layered epoch-1 architecture while refusing
to freeze that namespace-only wire. Existing-file replay, the persistent Q1
policy representation, portable-C parity and real-device qualification remain
M14 gates rather than unresolved architecture choices.

## 2026-09-02 — Shared extents and reflinks qualified

The ADR-061 implementation completed the volume-wide reference tree, shared
copy-on-write writes, `CloneFile`, byte-range `CloneRange`, checker
reconstruction and corruption cases, and every-write/every-flush crash
matrices. Review caught and fixed the dangerous rc=2→1 case before closure:
removing one of two mappings deletes the reference record but never retires
the surviving file's blocks. A second review found a left-boundary prefetch
case that could leave adjacent equal-count records non-canonical; its
regression now exercises rc=2/rc=3→rc=2 merging.

When the original implementation session reached its limit, the remaining
range-clone lot was taken over in the shared checkout. The completed operation
shares every representable full block, privately copies partial boundaries,
keeps holes sparse and publishes both file maps with the reference tree in one
checkpoint. The same semantics are exposed through the portable VFS with
per-volume capabilities. The retained commands, revisions and limitations are
in [the completion report](implementation/shared-extents-completion.md).

## 2026-08-31 — Team board adopted

Coordination between agents and the owner moves to the shared
[agent-board](https://github.com/jonx/agent-board) (project `afsplus`).
`board init . --agents claude` wrote [.mcp.json](.mcp.json), the Claude Code
hooks under `.claude/` and the protocol block in [CLAUDE.md](CLAUDE.md);
[AGENTS.md § Team board](AGENTS.md#team-board) points to it. The board is the
live conversation; this file stays the repository's record.

## 2026-08-31 — Shared extents chosen as the next epoch-1 lot

With Mountable Alpha-0 closed, the next objective is the largest unimplemented
epoch-1 *requirement* rather than an open question: [ADR-027](adr/ADR-027-reflink-clones.md)
has been "Accepted as an epoch-1 format requirement" since the design phase, a
freeze gate depends on it, and nothing implemented it — the extent record
carried one flag and no reference state existed. ADR-027 itself records why
that cannot be deferred: sharing changes the allocator, checker, reclamation
and reverse-map invariants at once. It is also coupled to open question Q1,
because reflink-shared ranges always copy on write, so the write path must fork
on shared versus private — exactly where the data-policy decision lives.

[ADR-061](adr/ADR-061-shared-extent-references.md) selects the mechanism: a
volume-wide typed reference tree on the ADR-034 engine, keyed by physical run.
A first draft was revised after review found six contract gaps, two of them
genuine correctness holes: an operation on a flagged extent must partition by
overlap rather than look up its start, because peers split the underlying runs
independently; and although clone is not journalled, the existing journalled
delete and rename-replacement can remove a file holding shared extents, so
their replay has to maintain the counts. The review also refused the
checkpoint offset until the transitional inline region records — dead since
[ADR-035](adr/ADR-035-allocation-root-reserved-pool.md) but still decoded when
the allocation root is zero — were removed outright rather than left to
collide with a new field.

That review exposed a defect in these very rules: they declared ADRs immutable
from the moment they were written, which leaves no way to correct a draft that
review has rejected. The rule now says what it should have said — immutable
from acceptance, revisable while `Proposed`.

## 2026-08-31 — Documentation restructured around one home per fact

The repository's documentation is reorganised so that state, design,
procedure and history each have exactly one home
([README.md § Documentation map](README.md#documentation-map)), every document
is reachable from an index, and [`tools/check-docs.py`](tools/check-docs.py) enforces the mechanical
part of the contract (`make check-docs`).

Measured before the restructuring: 132 Markdown files (13,785 lines) outside
the code trees; 82 files carried a `Status:` line (60 ADRs and 22 others); 365
journey words ("now", "still", "remains", ...); 2 Markdown links between
documents against 118 backticked path mentions (9 of them not resolving from
the repository root); 59 documents referenced from nowhere; 31 files longer
than 150 lines, none with a table of contents; `FILE_INDEX.md` and
`SHA256SUMS.json` generated once in the initial commit and never again.

What moved where:

- [`implementation/milestones.md`](implementation/milestones.md) is the only status authority. [`README.md`](README.md)
  keeps a five-row summary; [`ROADMAP.md`](ROADMAP.md) carries the stage plan only and links
  each stage to its milestones.
- `Status:` lines outside `adr/` were removed. Where a line carried a
  substantive sentence (docs 23–26, the fsync baseline) the sentence stays as
  plain text; the classification words are recorded here:

| File | Removed line |
|---|---|
| [`ROADMAP.md`](ROADMAP.md) | Status: initial review complete. Continue subsystem-by-subsystem only when implementation reaches that subsystem. |
| `CODEX_HANDOVER.md` | Status: active handover document for continuing architecture supervision and implementation review. |
| [`docs/23-pfs3-stage0-review.md`](docs/23-pfs3-stage0-review.md) | Status: architecture review v1, completed before the first implementation milestone. Source-level study should continue when the corresponding AFS+ subsystem is implemented. |
| [`docs/24-filesystem-comparison.md`](docs/24-filesystem-comparison.md) | Status: design benchmark. This table compares architectural capabilities, not marketing claims. AFS+ entries marked Planned or Proposed are not implemented yet. |
| [`docs/25-filesystem-wishlist.md`](docs/25-filesystem-wishlist.md) | Status: product/design exploration. Nothing in this document is automatically a 1.0 requirement unless promoted by an ADR. |
| [`docs/26-debug-observability.md`](docs/26-debug-observability.md) | Status: required development architecture. Most facilities are runtime-optional and must have near-zero cost when disabled. |
| [`docs/27-rust-implementation-strategy.md`](docs/27-rust-implementation-strategy.md) | Status: implementation direction |
| [`docs/28-virtual-images-and-viewports.md`](docs/28-virtual-images-and-viewports.md) | Status: developer-platform design |
| [`docs/29-first-class-content-inspection.md`](docs/29-first-class-content-inspection.md) | Status: developer/security API design |
| [`docs/30-portable-security-model.md`](docs/30-portable-security-model.md) | Status: security architecture proposal |
| [`docs/31-extreme-workloads.md`](docs/31-extreme-workloads.md) | Status: workload architecture and qualification plan |
| [`docs/32-reflink-clone-semantics.md`](docs/32-reflink-clone-semantics.md) | Status: epoch-1 design requirement for shared data extents |
| [`testing/extreme-workload-benchmarks.md`](testing/extreme-workload-benchmarks.md) | Status: required qualification design |
| [`testing/security-model-conformance.md`](testing/security-model-conformance.md) | Status: required test plan before security-format freeze |
| [`testing/aros-system-volume-qualification.md`](testing/aros-system-volume-qualification.md) | Status: required after Mountable Alpha-0 secondary-volume qualification |
| [`testing/benchmark-contract.md`](testing/benchmark-contract.md) | Status: required benchmark policy |
| [`testing/security-scanning-benchmarks.md`](testing/security-scanning-benchmarks.md) | Status: qualification requirement |
| [`implementation/peer-review-prototype-plan.md`](implementation/peer-review-prototype-plan.md) | Status: implementation guidance after external review |
| [`implementation/fsync-intent-log-baseline.md`](implementation/fsync-intent-log-baseline.md) | Status: measurement report for architecture blocker 2 (prefix removed, sentence kept) |
| `proposals/*.md` | Status: proposal — for team review … (replaced by a "Target on acceptance" line; the review request is stated once in [`proposals/README.md`](proposals/README.md)) |

- The M06 detail that lived in the milestone cell (which platforms are
  qualified by which gate) is now
  [testing/aros-system-volume-qualification.md § 7](testing/aros-system-volume-qualification.md#7-qualified-configurations).
- `FILE_INDEX.md` and `SHA256SUMS.json` are deleted: no generator, no
  consumer, stale since the first commit; Git and the per-directory indexes
  replace them.
- `CODEX_HANDOVER.md` (955 lines, written 2026-08-29 at commit `c2c2ed3`) is
  split and deleted. Where each section went: §1 role, §2 context precedence,
  §19 things not to do, §20 files to read first, §21 use of the conversation,
  §22 expected interaction and §24 resume sentence → [`AGENTS.md`](AGENTS.md); §3 what
  AFS+ is and §4 language strategy → the review lens in [`AGENTS.md`](AGENTS.md), the
  facts themselves being [README.md](README.md), [ADR-028](adr/ADR-028-rust-reference-core.md)
  and [ADR-029](adr/ADR-029-dual-reference-implementations.md); §5 current
  implementation, §6 lesson from the first crash matrix, §7 hardening list and
  §23 next action → the dated entries below; §8 architecture blockers and §13
  developer-idea maturity → [implementation/open-questions.md](implementation/open-questions.md);
  §9 allocator experiment → the design is [docs/07](docs/07-allocation.md) and
  [ADR-035](adr/ADR-035-allocation-root-reserved-pool.md), the G1/G2/G3 reuse
  workload with its allowed and forbidden states is
  [testing/crash-testing.md](testing/crash-testing.md#block-reuse-across-generations),
  its metrics are in [testing/benchmark-contract.md § 1](testing/benchmark-contract.md#1-benchmark-dimensions);
  §10 crash-consistency rules → the reviewer checklist in [`AGENTS.md`](AGENTS.md), the
  design being [docs/08](docs/08-transactions-and-journal.md) and
  [ADR-021](adr/ADR-021-deferred-reclamation.md); §11 feature evolution →
  [docs/09](docs/09-feature-framework.md); §12 derived versus authoritative →
  [ADR-012](adr/ADR-012-catalog-derived.md), [ADR-013](adr/ADR-013-change-stream-bounded.md);
  §14 workload philosophy and §15 benchmark contract →
  [docs/31](docs/31-extreme-workloads.md), [testing/benchmark-contract.md](testing/benchmark-contract.md);
  §16 debuggability → [docs/26](docs/26-debug-observability.md) and the
  regression-scenario rule in [`AGENTS.md`](AGENTS.md); §17 portability rules →
  [docs/14](docs/14-paths-and-namespaces.md), [ADR-017](adr/ADR-017-namespace-outside-format.md);
  §18 security philosophy → [docs/30](docs/30-portable-security-model.md),
  [ADR-031](adr/ADR-031-portable-security-acls.md); the superseded ideas →
  [adr/README.md § Superseded directions](adr/README.md#superseded-directions).
  Two ADRs gained relation lines the text already implied: ADR-009 is amended
  by ADR-020, ADR-042 by ADR-056 and ADR-057.

The [`README.md`](README.md) status section read as follows before it was reduced to the
five-row table (moved here verbatim from `README.md § Status`, commit
`624b6a8`):

> This repository now includes an executable Rust prototype, checker, portable
> VFS API, FUSE protocol adapter, packet-neutral AROS DOS adapter, a versioned
> AROS C/staticlib boundary, a cross-qualified native `DosPacket` translator,
> bounded trackdisk partition adapter and fully linked off-tree handler module.
> The host adapter and mount CLI can be built with
> `cargo build -p afsplus-fuse --features fuser-adapter --bin afsplus-mount`.
> Linux uses fuser's native mount path. On macOS, install macFUSE and build with
> `--features macfuse-mount`; the mount CLI selects macFUSE's user-space FSKit
> backend and the mountpoint must be an existing directory or a new direct child
> of `/Volumes`. The library is loaded at runtime, so ordinary workspace builds
> do not require a system FUSE installation. If macOS's File System Extensions
> switches are inert, use the diagnostic and reversible workaround in
> [`docs/macos-fskit-activation.md`](docs/macos-fskit-activation.md). Fuse-T's NFS transport is not a raw
> substitute for macFUSE's message channel; see ADR-040. The native AROS bridge
> and its cross-build qualification are documented in
> [`docs/aros-native-bridge.md`](docs/aros-native-bridge.md), ADR-042 through ADR-060.
> [`tools/check-hosted-aros-alpha0.sh`](tools/check-hosted-aros-alpha0.sh) now qualifies a bidirectional Hosted
> MacAROS → macFUSE → Hosted MacAROS round trip on one checked image. Hosted
> intent-log replay and the cumulative post-bootstrap S1 system-volume pivot are
> also qualified. The handler now supports standard AROS `Mount SHUTDOWN`
> termination, restart through the retained device node and a final
> `Assign DISMOUNT`. AFS+ is distributed as an external `L:` handler plus
> DOSDriver and does not require acceptance of AFS+ into the upstream AROS
> source tree; an upstream build recipe remains optional. Genuine defects found
> in generic AROS interfaces are still fixed with focused regression-tested
> patches and proposed upstream. AROS qualification images use the versioned
> Unicode 16 case-insensitive, spelling-preserving namespace; general-purpose
> images remain configurable and default to case-sensitive lookup. The native
> Apple-AArch64 pre-hardware QEMU gate now loads the external handler from a
> versioned retained image, executes the same operation matrix and proves a
> clean dismount/unload. File-backed guest RAM lets the host extract and
> strictly check the mutated payload; all six modeled intent-log cuts also
> replay to their exact old/new state with zero pending records. The RAM block
> transport still makes no reset-durability or Apple-hardware claim; those
> remain explicit gates. Autonomous boot selection and the classic ports also
> remain later gates. Qualification is reported as three target platforms over
> four ordered stages: Hosted MacAROS, native MacAROS/Apple Silicon, Amiga
> 500/m68k emulation, then the physical A500. The emulator is the pre-hardware
> validation stage of the A500 target. [`tools/check-aros-m68k-boot-fsuae.sh`](tools/check-aros-m68k-boot-fsuae.sh)
> now makes the first native m68k boot prerequisite machine-readable with
> matching official ROM, floppy and system-media hashes.
> [`tools/check-aros-m68k-alpha0-fsuae.sh`](tools/check-aros-m68k-alpha0-fsuae.sh) now also qualifies the real external
> handler's Alpha-0 operation matrix and all six recovery cuts on both the
> M68020-or-newer reference profile and an A500-configured plain M68000
> profile. The latter rejects M68020 long-multiply instructions before boot and
> binds the patched LLVM library in its evidence. This deliberately makes no
> physical-A500, final constrained-memory-budget or hardware-performance claim;
> ADR-057 records the boundary. ADR-058 adds a measured 8-MiB emulator profile
> and proves that sequential shutdown/restart cycles reuse one DOS-loaded
> segment without material memory growth; 4 MiB does not boot the control AROS
> profile far enough to test AFS+. ADR-059 makes AROS fatal diagnostics an
> explicit verdict across Hosted, native-QEMU and FS-UAE gates, so a modal
> requester cannot be mistaken for a hang or a successful outer process.
> ADR-060 closes the Mountable Alpha-0 objective with
> [`tools/check-mountable-alpha0.sh`](tools/check-mountable-alpha0.sh): one checksummed composite result binds the
> portable APIs, real macFUSE round trip, Hosted/native operation matrices and
> twelve recovery cases. This completion makes no physical-hardware or format
> freeze claim.

Two facts from that paragraph found finished-state homes rather than this
journal: the mount-CLI build instructions are in
[README.md § Build, test and mount](README.md#build-test-and-mount), and the
external-handler distribution model is stated in
[README.md § What AFS+ is and is not](README.md#what-afs-is-and-is-not) with
[ADR-050](adr/ADR-050-external-aros-handler-lifecycle.md). The rule that
genuine defects in generic AROS interfaces are fixed with focused
regression-tested patches and proposed upstream is retained in [AGENTS.md](AGENTS.md).

## 2026-08-31 — Mountable Alpha-0 closed

[`tools/check-mountable-alpha0.sh`](tools/check-mountable-alpha0.sh) produced `result=PASS` with
`hardware_claim=none`: the portable VFS, FUSE protocol, intent-log and AROS
adapter suites, one real macFUSE/Hosted same-image round trip with clean
checkers at every boundary, six Hosted and six native replay cases (four exact
old states, two exact new states), no guest failure requester, and a
156-file checksum manifest that validated in full. The evidence set was
re-verified independently the same day in the gate's reuse mode. ADR-060
records the decision; M08 is complete; M06 stays partial because no physical
hardware was involved.

## 2026-08-29 — Region allocator, bounded mount and shared COW trees

(Moved from `CODEX_HANDOVER.md` § 5 and § 23; the commits are `7a61c06`
through `35ec483`.)

Commit `7a61c06` completed the immediate hardening and the first Stage 6
allocator experiment: checkpoint selection is structural and never falls back
over corrupt state; same-generation slots are ambiguous; the negative
bad-ordering crash test is present; triple-buffered region descriptors and
three reserved generational slots per logical bitmap page break allocator
self-reference; retired blocks spend one generation in quarantine before
reuse; create-with-content, delete, the G1/G2/G3 reuse crash workload and
resource measurements are implemented. The first major proof after the
bootstrap prototype was not "allocation works" but "AFS+ can reuse storage
after deletes without any selectable checkpoint ever observing stale metadata
that points at newly reused content" — that invariant passed the modeled crash
matrix.

The continuation replaced the eager mount walk with bounded root loading.
Ordinary mount reads identification/checkpoints, the object-map root, root
object/directory-tree root and the retired-list root; descendant records are
decoded on access and allocation bitmaps are split across independently
checksummed pages, loaded as a mutation touches them through the selected
region descriptor. Clean pages that fail an allocation scan are evicted
immediately; the checker retains the exhaustive whole-volume view.

The Stage B1 continuation supports the proposed 262,144-block (1 GiB at 4 KiB)
region. The verifiable wire representation needs nine bitmap pages, not the
earlier rough estimate of eight. Three region-descriptor slots plus three slots
per bitmap page reserve 30 blocks (120 KiB, about 0.0114 %) per full region;
region 0 additionally contains ident and the two checkpoints. A small
transaction writes one dirty bitmap page and one descriptor before the
checkpoint. Cross-page allocation, corruption deferral/detection, quarantine
and power-cut behaviour are covered by executable tests. The multi-page design
deliberately does not put one record per bitmap page in the checkpoint: a
triple-buffered reserved region descriptor, itself selected by the
checkpoint's one record per region, binds the bitmap pages
([docs/07 § 3.1](docs/07-allocation.md)).

Core Scale-1 began with ADR-034 and the shared `AFST` node format: tree kind,
owner, level, strict binary keys, exact subtree item count and a counted
reference for every internal child, so an internal split can be planned from
the current page without reading every child. Transactional multi-upsert
copies a committed path once, keeps later changes in a write overlay, performs
balanced leaf/internal splits and grows the root; deletion merges or
redistributes underfull siblings and collapses one-child roots. A permuted
300-key test grows three levels, then deletes 299 keys in another permutation
and returns to one root leaf; a bad-level corruption test is executable. The
object-map adapter became authoritative (mkfs/checkpoints, bounded mount/stat,
mixed create/delete, checker ownership, crash matrices, a typed 1,001-entry
multi-page test), then the directory adapter (typed leaf at mkfs, bounded
lookup, exhaustive enumeration, 1,000-entry and end-to-end 300-entry
checked/remounted tests). Legacy object-map and directory codecs stayed as
transitional tests.

The namespace continuation removed the root-only API restriction: directory
lookup/enumeration, file creation/deletion, mkdir/rmdir, hard links and
same/cross-directory rename address parents by stable object ID. Rename
publishes both parent trees, parent records, the moved record and object-map
updates in one checkpoint, preserves object identity and rejects moves into
self or descendants; its every-write/every-flush matrix accepts only the
complete pre- or post-rename namespace. Link counts are transactional; the
first unlink preserves shared data and the final unlink retires it.

The typed extent-map adapter followed: small contiguous files keep the
zero-extra-tree direct representation; fragmented, sparse or preallocated
files set an experimental object flag and use an owner-bound `AFST` tree keyed
by logical block. `write_file_at` uses full data COW and supports writes beyond
EOF, `truncate_file` preserves zero-tail semantics, `preallocate_file` creates
zero-reading unwritten mappings, and final unlink retires both extent-tree and
mapped data blocks. A dedicated matrix proves sparse range writes recover to
exactly the pre- or post-transaction content.

The multi-upsert overlay retains every dirty node in RAM by default but has a
constrained mode with a 2/4/8-page final-image LRU that spills to
allocated-but-unreachable blocks; all three budgets pass a multi-level mutation
test, and a 100,000-key eight-page structural qualification passes (about 8.1 s
optimised, 53 s debug, so it is an explicit ignored scale test). This did not
close the classic-memory gate: decoded recursive ancestors and the split peer
stayed outside the measured staged-image budget.

ADR-035 fixed the allocation-root self-reference direction: `AFST` nodes in a
permanently allocated `3N` pool for an `N`-node fixed-topology region tree,
leaving a complete writable generation while two checkpoints stay selectable.
`TreeAllocator` decoupled the COW engine from ordinary free-space allocation;
inline checkpoint records became empty. A 1 TiB geometry bulk-build creates
1,024 typed records; a sparse 1 TiB image formats, bounded-mounts, performs two
small commits and runs the exhaustive checker in about 3.51 s on the
Apple-Silicon/APFS development host (commits about 18.6/12.0 ms, checker about
2.34 s), with host physical allocation between about 112 MiB and 2.13 GiB, so
the test uses a relative < 1 % sparse bound. Allocation-root publication
upserts only dirty region records (at most three tree nodes per measured
commit); the 145-region boundary forces a two-level allocation root and has an
exhaustive crash matrix; checker bitmap equality compares a sparse accounted
set against set bits byte-wise.

The next actions recorded at that point were: prototype multi-page region
bitmaps and larger regions against the one-page baseline, and measure the
fsync checkpoint path before deciding whether an auxiliary durability log is
justified — both done later the same week
([fsync-intent-log-baseline.md](implementation/fsync-intent-log-baseline.md),
[ADR-036](adr/ADR-036-reclaim-queue.md), [ADR-037](adr/ADR-037-intent-log.md)).

## 2026-08-29 — Lesson from the first crash matrix, and the hardening list

(Moved from `CODEX_HANDOVER.md` § 6 and § 7.)

Because the bootstrap allocator never reused blocks, an incorrectly ordered
commit could sometimes be hidden by doing a full reachable-state validation at
mount and falling back to the old checkpoint. That is dangerous: the crash
harness appears to prove ordering correctness while mount is repairing around
an invalid commit protocol. `validate_checkpoint_reachable()` also walked
every object reachable from the checkpoint; used as the ordinary selection
path it would turn mounting a volume with millions of files into a full-volume
scan. The design direction that followed — bounded structural checkpoint
selection at mount, full reachable validation only in `afsplus-check`, shadow
verification, the test harness and explicit recovery modes; torn or
CRC-failing checkpoints as the only legitimate fallback — is the rule the
reviewer checklist in [`AGENTS.md`](AGENTS.md) carries.

The hardening list executed before the allocator work: separate bounded
checkpoint selection from full reachable validation and add the negative
mis-ordered-commit crash test; make `Timespec::read()` and every public decode
helper reject short buffers instead of panicking; checked arithmetic for
generation, next free block, next object ID and range arithmetic; two distinct
checkpoint slots claiming the same generation are ambiguous, not tie-broken;
the power-cut model is described exactly as what it enumerates (all
full-write subsets plus representative torn writes of the modeled tail);
directory entries enforce `entry.key == comparison_key(entry.name)`; an object
in the authoritative object map with zero namespace references fails
validation until an explicit orphan model exists.

## 2026-08-29 — First executable prototype

(Moved from `CODEX_HANDOVER.md` § 5; commit `74b1410`.)

Steps 1–5 of
[implementation/peer-review-prototype-plan.md](implementation/peer-review-prototype-plan.md)
became executable and green: 29 tests, zero clippy warnings, about 3,592
lines, `afsplus-format` building as `no_std + alloc` on
`aarch64-unknown-none`. `afsplus-format` held the prototype wire codecs
(explicit little-endian, no native-struct serialisation, CRC32C, a 32-byte
common metadata header with type, version, owner, generation, payload length
and checksum, bounds-first decoding, a normalisation-preserving directory
entry shape with an identity comparison-key encoder). `afsplus-block` held
the block provider plus memory, sparse file, trace/accounting, deterministic
fault and power-cut recording backends. `afsplus-core` held mkfs, the
identification record, alternating checkpoints A/B, bootstrap bump allocation
from a checkpoint high-water mark (scaffolding that never reused blocks), the
root object, root directory and object map, the first COW metadata
transaction (create empty file: four fresh metadata blocks, flush, alternate
checkpoint, flush) and the shared reachable-state validator. `afsplus-check`
was a verify-only checker reporting both checkpoint slots as text and
versioned JSON.
