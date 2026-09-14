# Audit implementation queue and handoff

This queue preserves the complete findings from the
[book review](../docs/33-practical-filesystem-design-review.md) and
[qualification coverage map](../testing/book-review-qualification.md#normative-coverage-map).
Implementation status belongs in [milestones](milestones.md); decisions belong
in [open questions](open-questions.md). A linked test proves only its stated
oracle and backend. Snapshot work does not replace the other rows.

<!-- toc -->

- [Resume here](#resume-here)
  - [Snapshot and recovery integration context](#snapshot-and-recovery-integration-context)
- [Milestone closure audit order](#milestone-closure-audit-order)
- [Complete work queue](#complete-work-queue)
- [Boundaries to preserve](#boundaries-to-preserve)
- [Verification and delivery](#verification-and-delivery)

<!-- /toc -->

## Resume here

Prioritize completing usable roadmap stages. Reconcile each stage and milestone
with implementation evidence; distinguish implemented capability, integrated
consumer and platform qualification. Keep ongoing review visible and excluded
from finite completion calculations. Preserve every requirement in the complete
queue below, including explicit native hardware and user-decision gates.

Use the [milestone closure audit order](#milestone-closure-audit-order) to select
the next finite prerequisite. For Stage A, qualify bounded overlay branches after
the slice requirement, then close the replay-artifact, resource-accounting and
cache-profile gaps recorded in [milestones](milestones.md#stage-a-executable-core-audit).
Update specs and status, validate and commit each completed unit, and push to the
existing private origin. Do not resume open-ended feature expansion merely because
it follows the previous implementation thread.

### Snapshot and recovery integration context

Continue Q4 with an integrated ownership/reclaim design, using the
[snapshot accounting model](../testing/book-review-qualification.md#snapshot-accounting-model)
as the regression baseline. Persistent consistent snapshots are the accepted
direction under [ADR-069](../adr/ADR-069-consistent-snapshots-first.md) and
[ADR-070](../adr/ADR-070-persistent-snapshot-priority.md).

Integrate and qualify [ADR-071](../adr/ADR-071-snapshot-lifetime-prototype.md): lifetime
metadata across reflinks and last-live-reference retirement, plus a bounded
persistent traversal that can pass protected queue entries;
measure metadata reserves and write amplification. Qualify the persistent traversal and its resource limits against the model. Keep snapshot-owned namespace
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
with the [PAX archive/restore direction](../adr/ADR-076-pax-backup-interchange.md)
and [archive qualification](../testing/backup-archive-qualification.md), using
[envelope receipts](../spec/backup-envelope.md) for integrity/termination and
separate preservation-profile validation,
preserving explicit work limits before
recovery. Use the [metadata restoration primitive](../testing/book-review-qualification.md#metadata-restoration-primitive)
and the [checked destination interface](../docs/13-filesystem-api-v2.md#9-destination-scoped-restore-extension).
Define existing-destination merge/overwrite/resume under Q11 before exposing
objects present before the restore job; distinguish partial restoration from completion.
Use the [sparse content consumer](../spec/backup-sparse.md) for bounded content
transport and the [allocation-preserving consumer](../spec/backup-allocation.md)
for reservation binding and verified coverage. Use [bound regular-file groups](../spec/backup-file.md)
for exact metadata/content/inventory agreement. Use [directory and alias groups](../spec/backup-namespace.md) as namespace
components; integrate symlinks and the complete hard-link graph. Use the
[core symlink and retained-target gates](../testing/symlink-qualification.md#core-namespace-and-retained-target-gates)
for namespace storage and [scoped target transport](../docs/13-filesystem-api-v2.md#scoped-symlink-target-transport)
for VFS unlink dispatch, grant-held captured reads and destination creation.
Use [bound symlink archive groups](../spec/backup-namespace.md#symlink-target-preservation)
for target/profile/metadata binding and exact readback; integrate them into the
complete namespace job before claiming whole-backup preservation; use [scoped created-entry lookup](../docs/13-filesystem-api-v2.md#scoped-created-entry-lookup)
to reopen entries with bounded active handles and finalize metadata after namespace edits. Persist explicit
content-recovery loss reports and validate every object and archive-wide binding
before full-job preservation success. Use
[committed destination allocation readback](../docs/13-filesystem-api-v2.md#committed-destination-allocation-readback)
to compare restored coverage and written/unwritten state, rather than trusting
reservation success or allocated-byte totals alone.
Use [captured allocation enumeration](../docs/13-filesystem-api-v2.md#captured-allocation-enumeration)
for source ranges and the [destination reservation gate](../testing/security-model-conformance.md#16-destination-reservation-restoration)
for restoration. Use [private reservation initialization](../testing/data-policy-qualification.md#private-unwritten-reservation-initialization)
as the consumption baseline; qualify sustained near-full workloads, metadata
headroom and bounded traversal of fragmented existing layouts, using the
[bounded reservation gate](../testing/data-policy-qualification.md#bounded-reservation-edits)
and [bounded write gate](../testing/data-policy-qualification.md#bounded-writes)
for local edits, and [sparse growth](../testing/data-policy-qualification.md#sparse-growth)
for size extension. Use [bounded shrinking](../testing/data-policy-qualification.md#bounded-shrinking)
for admitted tail retirement; implement persistent bounded cleanup for arbitrary
large atomic truncation before qualifying unrestricted constrained resizing.
Use [explicit object metadata and inventory knowledge](../spec/backup-object-metadata.md)
for preservation admission; an unavailable enumeration API must not imply an
empty attribute or security inventory. Use [opaque captured transport](../docs/13-filesystem-api-v2.md#opaque-captured-metadata-transport)
and [staged destination publication](../docs/13-filesystem-api-v2.md#staged-opaque-metadata-restoration)
for the authorized consumer, with [bound opaque archive pairs](../spec/backup-opaque-values.md)
and publication gated on verified input from the original reader. Use
[inventory manifests](../spec/backup-inventory.md) and
[verified scratch replay](../spec/backup-spool.md) for bounded incremental
publication. Implement and qualify the AFS+ storage mapping, complete inventory
matching and large-inventory workloads, including segmented scratch beyond
host file/seek limits. Complete attribute/security archive transport
with explicit unsupported-state refusal under [ADR-078](../adr/ADR-078-backup-preservation-modes.md); qualify native host grant issuance. Preserve [ADR-074](../adr/ADR-074-protect-previous-checkpoint.md)
with both-slot crash oracles, generation-aware quarantine and measured
low-space progress. Integrate and qualify [ADR-075](../adr/ADR-075-revocable-backup-capability.md)
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

## Milestone closure audit order

Before extending the consumer feature set, reconcile each milestone's stated
exit criterion with its implementation, integration and platform qualification
evidence in [milestones](milestones.md). Preserve all queue requirements below.
Stage completion must be supported by the stage's own usable outcome.

1. Inspect executable-core evidence for M03, simulated publication/recovery for
   M04, and corruption detection for M05. Distinguish additional salvage and
   repair requirements from detection, and physical durability from simulated
   crash correctness.
2. Separate reader implementation from the format-freeze requirement in M00,
   retaining all epoch-1 freeze requirements under the final qualification order.
3. Record capability, integrated-consumer and platform gates in the status home,
   with component-owned evidence and explicit unverified findings. Reconcile
   stage-marker generation with those gates before changing completion labels.
4. Resolve the [Stage A finite gaps](milestones.md#stage-a-executable-core-audit):
   bounded slice wrapper, isolated overlay with explicit durability semantics,
   artifact replay and CPU/RAM accounting. These feed the existing publication,
   mixed-I/O, resource-pressure and sustained-qualification rows below.
5. Select subsequent implementation units by the missing prerequisite for a usable
   stage outcome; retain whole-job backup/restore and every other queue item.

## Complete work queue

Ordering expresses integration dependencies. Independent failure tests and
adapter work can proceed while a format question is discussed.

| Work item | Owner and prerequisite | Existing evidence to inspect | Next action and completion gate |
|---|---|---|---|
| Fragmented allocation search | M03; ordinary allocator | Core `fragmented_allocation` test and allocation counters | Preserve fallback regression; measure residual within-region rescans on aged application workloads before further optimization. |
| Uncertain publication | M03/M04; common commit engine | `faults` final-barrier, completed-write and adoption-read regressions | Preserve remount-required mutation rejection; extend each new publication path to the same oracle. |
| Internal diagnostic coverage | M01/M03, Stage A; common commit engine and replay harness | [Common checkpoint-tail ring](../testing/developer-harness.md#common-checkpoint-tail-flight-recorder), bounded storage and publication-failure tests | Integrate API-wide identities, category selection, live callbacks, allocator/tree/cache/intent-log/recovery events; extend the [versioned commit-tail replay export](../testing/developer-harness.md#internal-diagnostic-bundles) to those subsystems and verify event loss and correlation under faults without changing filesystem outcomes. |
| Mixed I/O correctness | M03/M05; file/extent operations | `streaming_api` three-seed byte oracle | Extend with each new storage representation; preserve sparse, unwritten, truncate and clone isolation through remount. |
| Repeated resource pressure | M03/M13; reclaim and sharing | Shared-survivor baseline and [24-cycle retained-view fixture](../testing/allocation-qualification.md#repeated-retained-view-pressure) | Preserve exact live/historical bytes and both-slot verification with one/eight-record scans; extend to aged multi-region sustained consumers and full memory accounting, building on the [phase-boundary RSS/read series](../testing/benchmark-contract.md#phase-boundary-resident-memory-and-repeated-reads) and [allocation-origin accounting](../testing/benchmark-contract.md#allocation-origins-and-instrumentation-cost), without treating a short read series as sustained qualification or allocation origin as current ownership. |
| Persistent snapshot views | Q4, M14; retention experiment, then accepted registry encoding | ADR-071/072/073; snapshot trees, allocator and Volume orchestration gates | Extend registry, ownership, read-view and release integration with full-consumer multi-view oracles, reboot and create/delete crash coverage. |
| Retention and accounting policy | Q4; same workload for both candidates | Retirement-generation queue and fixed allocation pool | Integrate and measure lifetime accounting plus traversal past protected entries; measure admission limits and capacity reporting; preserve the accepted busy-on-active-handle deletion rule. |
| Salvage and extraction | Q11, M05; damage classification | [Extraction corpus](../testing/extraction-qualification.md), `corruption_corpus`, `mount_modes` | Define supported damage classes; extract to a separate destination with explicit missing/untrusted-data report and zero source writes. |
| Repair | Q11, M05/M14; salvage corpus and accepted repair operations | Checker invariants; recovery design | Implement selected repairs transactionally; report identities/actions/loss; crash each repair boundary and verify exact allowed outcomes. |
| Backup and restoration | Q11, M13; stable snapshot view for online consistency | Scoped restore and bound file/directory/alias archive groups; backup archive qualification | Integrate whole-job archive/restore, symlinks and complete hard-link graph; compare names, bytes, sparse allocation, timestamps, attributes and security metadata. |
| Adapter lifetime/concurrency | M07/M13; host API lifetime contract | VFS open-unlinked, replacement, hard-link and no-changes tests | Exercise concurrent read/write/append/close/unmount and independent versus duplicated state; retain in-flight resources and bound cancellation. |
| Cache and VM integration | Q12, M07/M13; adapter/provider contract | Host block-fault tests; cache invariants | Test bypass/buffered coherence, eviction, failed writeback, low-memory reentrancy and contention progress on actual adapters. |
| Device durability and lifecycle | Q12, M13/M14; native provider and authorized hardware procedure | Simulated cuts and injected faults | Qualify DMA completion, barriers, shutdown, sleep/wake and power interruption separately on the target; document provider limits. Hardware writes require discussion. |
| Catalog completeness | M09; consistent enumeration boundary | Catalog generation/link-row requirements | Implement preexisting-link backfill, concurrent catch-up, complete publication and interruption fallback; scan-oracle equality. |
| Secondary query behavior | Q8, M09; named consumer and predicate decision | Book predicate/duplicate corrections | Specify missing/type-mismatched values, Unicode, AND/OR/NOT, duplicates and cancellation; compare indexed results with independent scans under duplicate-heavy workloads. |
| Persistent change discovery | M10; correct enumeration fallback and cursor design | Stream handoff/reset invariants | Implement committed history, atomic handoff, reset identity, retention expiry and overflow/rescan; no silent gaps or fabricated history. |
| Security metadata preservation | Q5, M14; actual adapters and transport | Security-container proposal and conformance plan | Demonstrate unknown-metadata round-trip or explicit refusal; decide rich evaluation only after cross-platform mapping evidence. |
| Tiny files and metadata placement | Q6, M13/M14; representative source/package workload | Allocation and mixed-I/O baselines | Compare inline, packed and locality-based storage, including attribute transitions, failure rollback and amplification; record the selected mechanism. |
| Reconstruction profile portability | M01/M12, Stage D; retained source/dependency inputs and qualified host toolchain profile | [Copied-host reconstruction gate](../testing/developer-harness.md#copied-host-toolchain-and-sdk-qualification) | Repeat frozen offline builds and artifact comparisons on each supported host with explicit compiler, linker, SDK and runtime evidence; preserve same-host and cross-host scope separately. |
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
