# Notes

The project journal: what was decided, tried and delivered, newest first.
This is the only document that narrates. Everything else states the finished
state and links here for the story; see
[docs/DOCUMENTATION.md](docs/DOCUMENTATION.md) for the rules.

Entry format: `## YYYY-MM-DD — title`.

<!-- toc -->

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
