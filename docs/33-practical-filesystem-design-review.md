# 33. Practical File System Design review

> **ADRs:** [ADR-063](../adr/ADR-063-intent-log-epoch1.md), [ADR-067](../adr/ADR-067-epoch1-allocation-state.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [book-review qualification](../testing/book-review-qualification.md) · **Milestones:** M03, M04, M09, M10, M13, M14

<!-- toc -->

- [Source and scope](#source-and-scope)
- [Chapter coverage and disposition](#chapter-coverage-and-disposition)
- [Corrections to earlier interpretations](#corrections-to-earlier-interpretations)
- [Allocation improvement within the accepted architecture](#allocation-improvement-within-the-accepted-architecture)
- [Requirements before optional indexes and live queries](#requirements-before-optional-indexes-and-live-queries)
- [Transactions, lifetime and recovery](#transactions-lifetime-and-recovery)
- [Cache, resource and integration gates](#cache-resource-and-integration-gates)
- [Follow-up order](#follow-up-order)

<!-- /toc -->

## Source and scope

Dominic Giampaolo, *Practical File System Design: The Be File System* (1999),
[Haiku-hosted PDF](https://www.haiku-os.org/legacy-docs/practical-file-system-design.pdf).
Research date: 2026-09-13. The review covers all twelve chapters and the
construction-kit appendix, with bibliography/index used for navigation.
Page references below are printed pages; add ten for the PDF viewer page.
PDF SHA-256: `8ccde4a26047827f75da52f286a7711815641add6795cc3b5162ec9439871f47`.

AFS+ source baseline: `44ea5ebb7ba1efb5fd37588956700500f1047e1e`.
The book describes historical BFS, BeOS and contemporary competitors. It
cannot establish current Haiku behavior, SSD timing, device atomicity or AFS+
correctness. Its benchmark ratios are workload observations, not AFS+ targets.
Implementation results belong in [milestones](../implementation/milestones.md)
and the dated [journal](../NOTES.md).

## Chapter coverage and disposition

| Chapter and pages | Focus | AFS+ disposition and owner |
|---|---|---|
| 1, 1–8 | OS and application goals | Adopt workload-led evaluation; [goals](01-goals-and-nongoals.md), M13. Missing host capabilities become development work. |
| 2, 9–32 | Namespace, data and lifecycle | Adapt to stable object IDs, hard links, sparse extents and orphan cleanup; [objects](04-object-model.md), [extents](06-files-and-extents.md), M03/M07. |
| 3, 33–44 | Competing architectures | Preserve historical scope; [comparison](24-filesystem-comparison.md). No rankings inferred from old implementations. |
| 4, 45–64 | Structures and encoding | Keep explicit codecs, checked geometry and bounded trees; [format](03-on-disk-format.md), M05/M14. |
| 5, 65–98 | Attributes, indexes, queries | Add completeness, freshness and predicate gates; [catalog](10-global-catalog.md), M09. |
| 6, 99–110 | Allocation and placement | Adapt fallback to fragmentation; [allocation](07-allocation.md), M03. Physical placement remains advisory. |
| 7, 111–126 | Transactions and recovery | Keep one COW engine with intent replay; [transactions](08-transactions-and-journal.md), M04. Reject a second BFS journal. |
| 8, 127–138 | Cache and concurrency | Require coherence, bounded pinning and progress; [performance](20-performance.md), M07/M13. |
| 9, 139–154 | Measurement | Adopt multiple workloads and explicit durability equivalence; [benchmark contract](../testing/benchmark-contract.md), M13. |
| 10, 155–184 | Vnodes and notifications | Adapt lifetime and post-commit delivery; [API](13-filesystem-api-v2.md), [stream](11-change-stream.md), M07/M10. |
| 11, 185–202 | Public interfaces | Keep filesystem-neutral capabilities and separate names from open identity; [API](13-filesystem-api-v2.md), M07. |
| 12, 203–214 | Testing and diagnosis | Add reproducible mixed-operation oracles; [qualification](../testing/book-review-qualification.md), M03/M05/M13. |
| Appendix, 215–220 | Hosted construction kit | Retain block-provider image tests and independent layers; [portability](17-portability.md), M01/M12. |

## Corrections to earlier interpretations

1. **OR is not inherently a full scan.** Pages 95–96 distinguish case-insensitive
   substring matching from combining separately searchable OR branches. An
   exact-match OR can perform two index lookups. Predicate shape, collation,
   available indexes and deduplication determine its cost.
2. **Old duplicate performance is not a present-day BFS finding.** Page 89
   describes the contemporary implementation. AFS+ must measure duplicate
   distributions, including high-cardinality posting sets, before selecting
   a secondary-index representation.
3. **Alignment is a decoder contract.** Pages 61 and 72 identify alignment
   hazards. Explicit byte-based little-endian decoding already avoids native
   struct alignment assumptions. Do not add disk padding solely for AArch64.
4. **Journals can carry file data.** Pages 115–116 describe BFS metadata-only
   logging and overgeneralize from its fixed transaction capacity. AFS+ must
   specify bounded records and data-before-reference durability. Its existing
   version-3 intent records already cover file updates without storing an
   arbitrarily large user buffer in the log.
5. **COW has relevant antecedents in the book.** Pages 116–117 discuss
   log-structured versioning; page 136 describes cloned cache images. Neither
   specifies AFS+ reflinks, but saying the book has no related guidance loses
   useful lifetime and generation lessons.
6. **No constant-time allocation promise.** Bitmap operations may be cheap;
   finding a suitable free run still scans. Measure positions examined, reads
   and retries under fragmentation, rather than claiming O(1) allocation.
7. **Timestamps are not durable change identities.** The page-84 tie-breaking
   technique does not solve clock rollback, user-set timestamps or missed
   event history. Use content generations and explicit stream cursors.
8. **Application metadata must survive transport deliberately.** Pages 176–179
   motivate archive/restore tests. An archive format's name alone does not
   prove that a particular tool preserves xattrs, hard links and security
   descriptors. Round-trip exact bytes through the actual AROS tools.

## Allocation improvement within the accepted architecture

`allocate_extent_runs` reduces its local maximum request after a failed
large-run search. Subsequent fragments use that cap for this invocation.
Allocation only consumes free space during the invocation; no later fragment
can benefit from repeating a request that already failed globally. The cap
is intentionally conservative: it can choose smaller extents than another
exhaustive search would discover. It is not persisted or reused by another
invocation, so reclamation and transaction-local releases can restore larger
allocations immediately.

This policy needs one existing stack variable and changes no encoding,
checkpoint ordering, free-space authority or retained-generation protection.
`AllocStats::allocation_searches` and `bitmap_bits_examined` expose search
work. Bitmap positions are CPU-work counts, not device I/O counts. The
existing byte/read/write/flush counters remain necessary for end-to-end cost.
The allocator still restarts its bitmap scan within a region; the change
removes repeated oversized probes, not all possible quadratic scan behavior.
A cursor or free-run accelerator remains a measured follow-up if that residual
cost dominates an application workload.

## Requirements before optional indexes and live queries

The [catalog](10-global-catalog.md) owns generation-bound enumeration and
backfill. The [change stream](11-change-stream.md) owns persisted catch-up and
rescan. The two can support a query service, but neither implies a complete
query language or secondary attribute index.

Before M09 activation, build a catalog from preexisting files while creating,
renaming, linking, deleting and updating attributes. Require a snapshot or
an enumeration/change-boundary protocol proving completeness. Publish only
when every required change through the advertised watermark is accounted
for. An interrupted or stale build must fall back to authoritative traversal.
A new index must never quietly omit attributes written before its creation
(book pp. 82–83). Existing catalog link rows correctly distinguish object
identity from multiple names; do not replace them with BFS's single-parent
assumption.

A secondary-query proposal must specify missing attributes, type mismatches,
integer widths, floating-point NaNs/signed zero if supported, Unicode
comparison versions, prefix versus substring matching, AND/OR/NOT behavior,
duplicate results, cancellation and resource limits. Compare every indexed
result with an independently evaluated authoritative scan. Selectivity may
change execution order but cannot change truth. Start with a named consumer
and measured value; an unanswered gate is a research task, not a reason to
abandon useful search.

M10 needs atomic subscription plus initial enumeration, or a catch-up cursor
that closes their race. Expose committed events only; a callback must not
reenter a partially initialized object or deadlock on transaction locks.
Delivery must be bounded with explicit overflow, cancellation and rescan.
Reinitializing a stream must invalidate old cursors even if sequence numbers
repeat within the same filesystem UUID. Retention loss cannot be repaired by
pretending current state reconstructs old events.

## Transactions, lifetime and recovery

Checkpoint COW, exact crash-state checks, intent replay, shared-owner accounting
and a bounded orphan directory provide the transaction baseline. The
`write_and_truncate_replay_is_restartable_after_every_cut` test covers
recovery interrupted by another crash. Extend these mechanisms when a new
operation is added; a fresh journaling implementation is unnecessary.

Before expanding compound operations, reserve enough transaction resources or
reject before publication. Test failures after replacement-target removal,
late tree splits, extent conversion and log exhaustion. Check namespace,
bytes, ownership and promised fsync prefixes, not mount success alone.
Deletion of highly fragmented objects must continue through bounded orphan
cleanup. A failed ordinary call and a failed commit with uncertain device
completion need distinct caller contracts; remount/recovery decides the
latter's allowed durable state.

Reference counts alone cannot prove namespace connectivity: a disconnected
directory cycle can have matching incoming counts. Checker traversal must prove
reachability from the ordinary and internal orphan roots for every retained
view. A structurally decoded older checkpoint also does not prove its storage
was retained. [ADR-074](../adr/ADR-074-protect-previous-checkpoint.md) requires
preserving previous-checkpoint metadata and registered snapshots through slot
replacement; qualify both-slot crash outcomes and the added quarantine pressure.

Open identity, directory-entry identity and descriptor state are separate.
Adapters must distinguish closing one handle from releasing the last handle
or in-flight request. Qualification includes hard-link replacement,
open-unlinked writes, independent versus duplicated offsets, append
serialization, cursor invalidation and unmount with outstanding references.
Authorization must apply to object-ID access as well as pathname lookup.
The book's unimplemented security hook is a warning, not a feature to copy.

## Cache, resource and integration gates

Host cache policy cannot weaken checkpoint barriers. Buffered and bypass I/O
must agree on dirty data; shared readers must not see uncommitted mutable
metadata. Test read/write overlap, eviction during a transaction, failed
writeback, delayed completion and recovery. Bound dirty pins and provide
backpressure rather than unbounded buffering. Do not hold a global cache
lock across blocking I/O. Verify progress under contention instead of relying
on sleep-and-retry fairness.

For VM-backed native I/O, document which calls may allocate or fault while
holding locks; exercise memory pressure and reclaim reentrancy. Benchmark
metadata, streaming, random I/O and real applications on fresh and aged
images with stated memory and durability budgets. Book-era seek timings,
64 KiB bypass thresholds and fixed cache fractions are not target policies.
Native device durability and multiprocessor scheduling remain independent
hardware/integration gates.

## Follow-up order

1. M03/M05: executable allocation and mixed-I/O regression, existing corruption,
   near-full and crash suites; quantify residual scan cost if workloads expose it.
2. M07/M13: qualify adapter handle lifetime and cache/VM behavior under real
   concurrent applications; add support where the current host cannot provide it.
3. M09: complete authoritative enumeration/backfill and secondary-query semantics
   before advertising a query fast path.
4. M10: qualify enumeration-to-subscription handoff, epoch invalidation and
   overflow before claiming reliable incremental consumers.
5. M14: include transport round-trip, low-space soak, retained-generation and
   independent-reader evidence in format review. Short seeded tests do not
   replace long-running mixed-load or native hardware qualification.
