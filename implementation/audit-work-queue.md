# Audit implementation queue and handoff

This queue preserves the complete findings from the
[book review](../docs/33-practical-filesystem-design-review.md) and
[qualification coverage map](../testing/book-review-qualification.md#normative-coverage-map).
Implementation status belongs in [milestones](milestones.md); decisions belong
in [open questions](open-questions.md). A linked test proves only its stated
oracle and backend. Snapshot work does not replace the other rows.

## Resume here

Continue Q4 with an integrated ownership/reclaim design, using the
[snapshot accounting model](../testing/book-review-qualification.md#snapshot-accounting-model)
as the regression baseline. Persistent consistent snapshots are the accepted
direction under [ADR-069](../adr/ADR-069-consistent-snapshots-first.md) and
[ADR-070](../adr/ADR-070-persistent-snapshot-priority.md).

Implement [ADR-071](../adr/ADR-071-snapshot-lifetime-prototype.md): lifetime
metadata across reflinks and last-live-reference retirement, plus a bounded
persistent traversal that can pass protected queue entries;
measure metadata reserves and write amplification. The model's rotating queue
requires an explicit production representation. Keep snapshot-owned namespace
metadata separate from allocation/reclaim machinery governed by selectable
checkpoints. Use [ADR-072](../adr/ADR-072-snapshot-record-codecs.md) for leaf codecs and
[ADR-073](../adr/ADR-073-snapshot-checkpoint-roots.md) for checkpoint root binding.
Use [typed tree access](../testing/book-review-qualification.md#typed-snapshot-tree-access),
[lifetime transaction preparation](../testing/book-review-qualification.md#lifetime-transaction-preparation),
[allocator/quarantine binding](../testing/book-review-qualification.md#snapshot-allocator-and-quarantine-binding),
[Volume orchestration](../testing/book-review-qualification.md#volume-snapshot-orchestration)
and [exhaustive ownership checking](../testing/book-review-qualification.md#exhaustive-snapshot-checker)
and [previous-checkpoint retention](../testing/book-review-qualification.md#previous-checkpoint-retention)
and [explicit mount/recovery](../testing/book-review-qualification.md#explicit-snapshot-mount-and-recovery)
as component baselines. Integrate the [trusted backup extension](../docs/13-filesystem-api-v2.md#8-trusted-snapshot-backup-extension)
with the [PAX archive/restore direction](../adr/ADR-076-pax-backup-interchange.md),
preserving explicit work limits before
recovery. Use the [metadata restoration primitive](../testing/book-review-qualification.md#metadata-restoration-primitive)
and the [checked destination interface](../docs/13-filesystem-api-v2.md#9-destination-scoped-restore-extension).
Define existing-destination merge/overwrite/resume under Q11 before exposing
pre-existing destination objects; distinguish partial restoration from completion.
Use [captured allocation enumeration](../docs/13-filesystem-api-v2.md#captured-allocation-enumeration)
for source ranges and the [destination reservation gate](../testing/security-model-conformance.md#16-destination-reservation-restoration)
for restoration. Use [private reservation initialization](../testing/data-policy-qualification.md#private-unwritten-reservation-initialization)
as the consumption baseline; qualify sustained near-full workloads, metadata
headroom and bounded traversal of fragmented existing layouts, using the
[bounded reservation gate](../testing/data-policy-qualification.md#bounded-reservation-edits)
for local reservation edits; add attribute/security transport
with explicit unsupported-state refusal under [ADR-078](../adr/ADR-078-backup-preservation-modes.md); qualify native host grant issuance. Preserve [ADR-074](../adr/ADR-074-protect-previous-checkpoint.md)
with both-slot crash oracles, generation-aware quarantine and measured
low-space progress. Implement [ADR-075](../adr/ADR-075-revocable-backup-capability.md)
for revocable trusted-backup authority; keep ordinary-user historical access
and rich ACL decisions under Q5.
Use `tree::read_key_page` for bounded inclusive-key seeking; persist the next
physical key and make scan wrap explicit.

Validate registry transactions against ADR-072/073. Further format changes
require the decision and compatibility process in
[CONTRIBUTING](../CONTRIBUTING.md). Do not pin arbitrary old
allocation roots in the fixed `3N` pool. Snapshot protection must override
in-place write eligibility. The first consumer must enumerate and read a
consistent view after remount, with exact metadata and byte oracles.

## Complete work queue

Ordering expresses integration dependencies. Independent failure tests and
adapter work can proceed while a format question is discussed.

| Work item | Owner and prerequisite | Existing evidence to inspect | Next action and completion gate |
|---|---|---|---|
| Fragmented allocation search | M03; ordinary allocator | Core `fragmented_allocation` test and allocation counters | Preserve fallback regression; measure residual within-region rescans on aged application workloads before further optimization. |
| Uncertain publication | M03/M04; common commit engine | `faults` final-barrier, completed-write and adoption-read regressions | Preserve remount-required mutation rejection; extend each new publication path to the same oracle. |
| Mixed I/O correctness | M03/M05; file/extent operations | `streaming_api` three-seed byte oracle | Extend with each new storage representation; preserve sparse, unwritten, truncate and clone isolation through remount. |
| Repeated resource pressure | M03/M13; reclaim and sharing | `allocation_pressure::repeated_near_full_cow_and_reclaim_preserve_shared_survivors` | Add retained-view pressure to the 24-cycle baseline; require bounded progress and explicit admission failure. |
| Persistent snapshot views | Q4, M14; retention experiment, then accepted registry encoding | ADR-069/070; proposal S1–S4 | Build registry, protected ownership, read view and release; exact multi-view oracle after live mutation, reboot and create/delete crashes. |
| Retention and accounting policy | Q4; same workload for both candidates | Retirement-generation queue and fixed allocation pool | Integrate and measure lifetime accounting plus traversal past protected entries; measure admission limits and capacity reporting; preserve the accepted busy-on-active-handle deletion rule. |
| Salvage and extraction | Q11, M05; damage classification | [Extraction corpus](../testing/extraction-qualification.md), `corruption_corpus`, `mount_modes` | Define supported damage classes; extract to a separate destination with explicit missing/untrusted-data report and zero source writes. |
| Repair | Q11, M05/M14; salvage corpus and accepted repair operations | Checker invariants; recovery design | Implement selected repairs transactionally; report identities/actions/loss; crash each repair boundary and verify exact allowed outcomes. |
| Backup and restoration | Q11, M13; stable snapshot view for online consistency | Clone/mixed-I/O tests do not prove backup | Run a real archive/restore consumer; compare names, links, bytes, sparse semantics, timestamps, attributes and security metadata. |
| Adapter lifetime/concurrency | M07/M13; host API lifetime contract | VFS open-unlinked, replacement, hard-link and no-changes tests | Exercise concurrent read/write/append/close/unmount and independent versus duplicated state; retain in-flight resources and bound cancellation. |
| Cache and VM integration | Q12, M07/M13; adapter/provider contract | Host block-fault tests; cache invariants | Test bypass/buffered coherence, eviction, failed writeback, low-memory reentrancy and contention progress on actual adapters. |
| Device durability and lifecycle | Q12, M13/M14; native provider and authorized hardware procedure | Simulated cuts and injected faults | Qualify DMA completion, barriers, shutdown, sleep/wake and power interruption separately on the target; document provider limits. Hardware writes require discussion. |
| Catalog completeness | M09; consistent enumeration boundary | Catalog generation/link-row requirements | Implement preexisting-link backfill, concurrent catch-up, complete publication and interruption fallback; scan-oracle equality. |
| Secondary query behavior | Q8, M09; named consumer and predicate decision | Book predicate/duplicate corrections | Specify missing/type-mismatched values, Unicode, AND/OR/NOT, duplicates and cancellation; compare indexed results with independent scans under duplicate-heavy workloads. |
| Persistent change discovery | M10; correct enumeration fallback and cursor design | Stream handoff/reset invariants | Implement committed history, atomic handoff, reset identity, retention expiry and overflow/rescan; no silent gaps or fabricated history. |
| Security metadata preservation | Q5, M14; actual adapters and transport | Security-container proposal and conformance plan | Demonstrate unknown-metadata round-trip or explicit refusal; decide rich evaluation only after cross-platform mapping evidence. |
| Tiny files and metadata placement | Q6, M13/M14; representative source/package workload | Allocation and mixed-I/O baselines | Compare inline, packed and locality-based storage, including attribute transitions, failure rollback and amplification; record the selected mechanism. |
| Sustained application qualification | M13/M14; usable adapters; snapshot consumer where applicable | Short deterministic tests and fsync workload harnesses | Run seeded aged/near-full combined workloads and real Cargo/Git/editor/media/backup consumers; retain duration, resource metrics and failure artifacts. |

## Boundaries to preserve

- Keep normal mount bounded; exhaustive verification belongs to the checker.
- Never reuse or discard blocks reachable from a selectable or retained view.
- Preserve acknowledged durability; uncertain publication requires reconciliation
  or remount before another mutation.
- Full COW and opted-in in-place writes have different byte-failure contracts.
- Catalogs are rebuildable; lost change history requires reset and rescan.
- Keep on-disk encoding independent of Rust and application APIs filesystem-neutral.
- Missing platform support becomes owned implementation work. Native evidence,
  long stress and optional facilities must not be inferred from host test success.
- Under the owner's delegated design authority, choose the recommended option
  for unresolved questions, record alternatives and rationale in the decision
  and question owner, and continue implementation. Preserve explicit public
  communication and hardware-write boundaries.
- Qualify constrained/older-system profiles with bounded memory, bounded work
  and explicit admission limits. Degraded operation may reduce optional features
  or throughput; preserve filesystem invariants and report unsupported semantics
  or preservation losses. Verify actual target behavior separately from host models.
- Completion requires a requirement-by-requirement correctness and platform-fit
  review, including constrained profiles. Record missing evidence explicitly.

## Verification and delivery

Run the targeted oracle while developing. For Rust commits, run the repository
quality gate: `cargo fmt --all -- --check`,
`cargo test --workspace --all-features`, and
`cargo clippy --workspace --all-targets --all-features -- -D warnings`.
For documentation, run `make toc`, `make check-docs` and `git diff --check`.
Record actual results in [NOTES](../NOTES.md), and change milestone status only
when its complete acceptance gate is met.

Both repositories are private; commit/push to their existing origins is
authorized. Public communication and hardware writes require discussion.
Preserve unrelated changes and stage explicit paths.
