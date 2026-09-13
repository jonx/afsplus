# Practical File System Design qualification

> **ADRs:** [ADR-067](../adr/ADR-067-epoch1-allocation-state.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** `cargo test -p afsplus-core fragmented_allocation -- --nocapture`; `cargo test -p afsplus-check --test streaming_api` · **Milestones:** M03, M05, M09, M10, M13, M14

The [book review](../docs/33-practical-filesystem-design-review.md) records the
source, chapter coverage and architectural rationale. Results belong in
[milestones](../implementation/milestones.md) and [NOTES](../NOTES.md).

<!-- toc -->

- [Executable allocation regression](#executable-allocation-regression)
- [Executable mixed-I/O oracle](#executable-mixed-io-oracle)
- [Snapshot accounting model](#snapshot-accounting-model)
- [Snapshot record codecs](#snapshot-record-codecs)
- [Snapshot checkpoint extension](#snapshot-checkpoint-extension)
- [Bounded key cursor prerequisite](#bounded-key-cursor-prerequisite)
- [Typed snapshot tree access](#typed-snapshot-tree-access)
- [Lifetime transaction preparation](#lifetime-transaction-preparation)
- [Snapshot allocator and quarantine binding](#snapshot-allocator-and-quarantine-binding)
- [Existing complementary gates](#existing-complementary-gates)
- [Required future experiments](#required-future-experiments)
- [Normative coverage map](#normative-coverage-map)
- [Repeated low-space and reclamation gate](#repeated-low-space-and-reclamation-gate)

<!-- /toc -->

## Executable allocation regression

`volume::fragmentation_tests::fragmented_allocation_does_not_repeat_oversized_searches`
constructs a transaction over a freshly formatted 512-block image with 4 KiB
blocks, then occupies alternating ordinary blocks. This is an allocator-unit
fixture, not a committed user filesystem image. Request 64 blocks and require:

- exactly 64 one-block extents at the independently predicted free positions;
- contiguous logical coverage and no allocation of the occupied blocks;
- at most 70 searches: six failed halvings plus 64 successful allocations;
- after releasing adjacent transaction-local blocks, a separate 16-block
  request succeeds as one run, proving the fallback does not remain sticky.

Print allocation searches and bitmap positions examined. The test asserts
work rather than elapsed time so machine load does not cause false failures.
Compare full commit workloads separately through the existing
[allocation qualification](allocation-qualification.md); the unit fixture
makes no claim about write amplification or native throughput.

## Executable mixed-I/O oracle

`seeded_mixed_io_preserves_bytes_across_remounts_and_clones` runs three fixed
seeds (`0xBF5`, `0xDEADBEEF`, `0x12345678`), 192 operations per seed, over
8,192-block images. Each operation chooses an arbitrary byte offset within
32 blocks and a length up to two blocks plus three bytes. It mixes writes,
truncate shrink/growth and unwritten preallocation.

A byte vector computes expected content independently of extent mappings.
After every operation, compare every visible byte, a partial read and the
untouched destination-buffer suffix. After operation 63, retain a reflink
copy and verify its complete content after every subsequent mutation.
Every 16 operations, require an exhaustive clean checker result, remount and
compare contents again. Failure messages identify seed and operation.

This is bounded deterministic regression coverage. It does not prove
exhaustive state exploration, simultaneous-thread safety or native durability.

## Snapshot accounting model

Run `cargo test -p afsplus-core --test snapshot_retention_model -- --nocapture`.
The [isolated model](../crates/afsplus-core/tests/snapshot_retention_model.rs)
compares oldest-generation FIFO retention with birth/retirement lifetime
intersection and a rotating bounded scan. Captured addresses and independent
32-byte payload copies provide the immutable-view oracle. A deliberately
unsafe reuse case must trip that oracle.

The workload budgets are experiment-local: 64 and 256 logical data blocks,
one live block, one retained old block, four times capacity in unrelated
create/delete cycles, and at most eight queue entries examined per reclaim
call. The useful-work target is completion of every cycle with only one
retained block and capacity minus two free blocks. Test explicit allocation
failure without live-state changes, equality at birth/retirement boundaries,
three snapshot-release orders, eventual progress behind 32 protected entries,
and complete reclamation within `ceil(queued / 8)` calls after final release.

The model assumes atomic COW updates and a lifetime ending at the last live
reference. It excludes selectable-checkpoint delays, sharing transitions,
metadata blocks, allocation-root reuse, registry persistence and crashes.
Its scan rotates entries in memory; the production sealed FIFO cannot do
that without a new representation or rewrite protocol. Eight examined entries
also permit up to eight times the snapshot count in lifetime comparisons.

Printed counts measure logical capacity and queue work only. Host allocations
scale with modeled blocks, queued retirements and captured files per view.
There is no device I/O, so CPU timing, peak RAM, metadata amplification and
recovery costs require an integrated prototype. Neither candidate qualifies
for shipping from this gate. Preserve these cases when integrating registry,
last-reference tracking, forced COW and a crash-safe reclaim cursor.

## Snapshot record codecs

Run `cargo test -p afsplus-format --test snapshot_records` and
`cargo test -p afsplus-check --test mount_modes`. The
[record specification](../spec/snapshot-records.md) follows
[ADR-072](../adr/ADR-072-snapshot-record-codecs.md).

Require fixed expected bytes, explicit key/value endianness, ID exhaustion,
every truncated/oversized record length, each reserved byte, future and invalid
lifetimes, physical-range overflow and control bounds. Construct complete
checksummed registry/ledger leaf block images, cross-check their typed values,
and require corruption rejection across header, payload and padding mutations.
These are standalone metadata images; full snapshot-enabled volume images
belong to the integrated persistence gate.

Until complete ownership support is integrated, all Rust mount modes and the
checker must refuse the snapshot feature. The portable-C reader probe also
mutates a valid image's identification, reseals its checksum, and requires
`AFSPR_ERR_UNSUPPORTED` for the snapshot bit. Run
`make portable-c-gate` for that independent rejection contract. A codec test
never grants snapshot mount capability or proves create/delete crash safety.

## Snapshot checkpoint extension

Run `cargo test -p afsplus-format snapshot_checkpoint_extension` and the mount
mode gate above. [Independent checkpoint block images](../crates/afsplus-format/tests/fixtures/checkpoint-roots.json)
fix legacy and extended byte output; their bytes were assembled separately
from the Rust encoder using explicit offsets and a bitwise CRC32C loop.
Run `python3 crates/afsplus-format/tests/fixtures/generate-checkpoint-fixtures.py`
to check reproducibility without writes; `--write` regenerates the fixtures.

Require exact legacy output, two appended root fields, rejection of every
other payload length, short encoder buffers, zero/equal/out-of-geometry roots,
and valid-CRC selected-feature mismatches. A mismatched newest checkpoint must
produce an error even with an older structural candidate. The C rejection
fixture carries the snapshot feature and two extended checkpoints. Standalone
root references in these fixtures do not qualify the persistent tree writer.

## Bounded key cursor prerequisite

Run `cargo test -p afsplus-core key_page` and
`cargo test -p afsplus-core key_cursor`. A three-level, 1,024-record tree is
compared with an independent ordered-key scan for inclusive lower bounds,
gaps, branch transitions, zero limits and end-of-tree. Every page obeys the
requested entry limit; traced device reads match reported reads, with at most
`6 + ceil(limit / 8)` reads in the fixture and six raw/decoded node equivalents
at peak. Output records and allocator overhead are additional memory costs.
The test requires zero writes and flushes.

Between pages, delete earlier records and insert behind the cursor. The next
key page must preserve all successors; an explicit wrap must discover the
insertion. Reject visited-child count mismatches, future generations and cycles.
This prerequisite proves bounded key seeking. Transactional cursor persistence
and release crash tests belong to the integrated ledger gate.

## Typed snapshot tree access

Run `cargo test -p afsplus-core --test snapshot_trees`. Registry pages must
preserve sparse IDs and reject missing control records, wrong tree kind/owner,
future capture generations, exhausted-ID misuse and reserved object-map roots.
Lifetime pages validate their predecessor and one lookahead record, rejecting
overlap and unmerged equal lifetimes across both batch edges. Cursor persistence
is simulated between reads; actual atomic updates need the transaction gate.

The leaf fixtures require two reads for registry control plus a page, three
for lifetime control plus predecessor and page, and no writes or flushes.
Reported reads must equal the traced device count. Multi-level traversal bounds
are covered by the key-cursor prerequisite. Output includes generic key/value
storage and typed records, both proportional to the requested page limit;
traversal buffers are additional. Root counts are advertised counts with visited
paths checked, not an exhaustive proof of all subtrees or retained-block sums.

Compare the constant-memory allocation-pool envelope against enumerated pool
blocks across region sizes, region counts and partial tails. Also calculate the
envelope at the maximum supported region count without materializing that pool.
Namespace range checks exclude region headers and permanent allocator storage;
bitmap ownership, intent-log exclusion and cross-tree alias detection remain
obligations of the enclosing transaction/checker integration.

## Lifetime transaction preparation

Run `cargo test -p afsplus-core --test snapshot_lifetime_edits -- --nocapture`.
Use persistent AFST nodes with a test-only allocator and a per-physical-block
oracle. Require birth preservation across splitting and unordered retirement,
canonical coalescing, exact retained totals/cursor updates, and unchanged old
COW tree bytes. Reallocation after a ledger ownership gap must establish a new
birth; release and reuse in one edit must fail.

A 600-record sparse ledger's one-record retirement loads exactly the affected
record and its two neighbors. Require at most nine preparation reads and fewer
than twenty mutation device reads, with reported reads matching tracing. Report
loaded records, traversal page equivalents, written nodes and mutation page
peak; these are algorithmic resource counters, not host RSS measurements.

Place a protecting view on the second registry page. Release must fail; an
insufficient registry-scan budget must also fail. Test overlapping allocations,
missing/double retirements, same-transaction birth retirement, live/changed/double
transfers, invalid cursors, insufficient edit memory and a corrupt retained sum.
These preparation failures must cause no allocator calls, writes or flushes.

The fixture publishes tree images without Volume or bitmap ownership. It does
not prove integrated checkpoint cuts, quarantine delay, busy handles, namespace
immutability or reboot recovery. Those remain requirements of the integrated
snapshot gate; the format feature cannot be enabled on this evidence alone.

## Snapshot allocator and quarantine binding

Run `cargo test -p afsplus-core --test snapshot_allocator -- --nocapture`.
The fixture uses real region bitmaps/descriptors, allocation-root slots,
checkpoint selection and the segmented reclaim queue. It bootstraps snapshot
trees explicitly; this is not a supported conversion or mount path.

Require namespace allocation capture for ordinary and exact replay allocations.
Released and log-sacrificed new blocks must not acquire committed lifetimes;
sacrificed blocks remain quarantined. Housekeeping and ledger nodes must remain
allocated without recursively entering the ledger. Refuse unsealed finalization,
post-seal caller mutations and any reuse of an allocator whose lifetime seal
failed. Only internal finalization can add housekeeping after sealing. Report allocator
inline bytes for the target architecture; snapshot accounting is allocated only
for enabled transactions, with that heap state and allocator overhead additional.

After last-live retirement, storage stays allocated in the ledger and outside
ordinary quarantine. A transfer removes it from the ledger, queues it at that
publication generation and leaves its bitmap bit set. Exact reallocation becomes
possible only in a subsequent transaction and establishes a new birth.

Enumerate the power-cut model at every recorded transfer publication boundary.
The selected state must have either the original lifetime/retained total or the
new quarantine/retained total, with the allocation bit set in both cases. Require
both generations to occur. Remove the metadata barrier as a negative control;
the oracle must detect new-checkpoint publication with missing referenced state.
The model covers full-write subsets and representative tears, not every physical
reordering. These allocator images do not qualify file-namespace snapshots,
registry transactions, read handles or native devices.

## Existing complementary gates

| Concern | Executable suite | Required observation |
|---|---|---|
| Recovery interrupted again | `afsplus-check --test intent_log` | Every modeled replay cut preserves the allowed durable prefix. |
| Late namespace failure and replacement | `afsplus-check --test batch` | Failed batches preserve the pre-state; successful replacement is atomic. |
| Near-full progress | `afsplus-check --test allocation_pressure` | Failed growth preserves state; delete/reclaim restore usable space. |
| Open-unlinked lifetime | `afsplus-check --test orphans --test intent_replay_orphans`; VFS `api` | Stable open identity and bounded restartable cleanup. |
| Invalid disk references | `afsplus-check --test corruption_corpus` | Reject malformed or valid-CRC semantic corruption at the specified boundary. |
| Shared block ownership | `afsplus-check --test shared_crash --test shared_extents` | No modification of another owner or premature physical reuse. |

The names in this table are Cargo package/test selectors, not standalone shell
commands. Run them as `cargo test -p PACKAGE --test TEST`.

## Required future experiments

| Owner and order | Experiment | Pass condition and consequence of failure |
|---|---|---|
| M09, catalog before activation | Preexisting attributes plus concurrent link/rename/delete during backfill; crash each publication boundary | Exact authoritative enumeration at the advertised generation; otherwise discard build and use traversal. |
| M09, secondary-query proposal | Missing/type-mismatched attributes; high duplicate counts; exact OR, prefix, substring and Unicode cases | Scan-oracle equality with bounded resources; otherwise change predicate/index design before exposing it. |
| M10, stream before incremental apps | Enumeration/subscription race, slow consumers, overflow, retention expiry and reset | No silent gap; explicit rescan and old-cursor rejection; otherwise repair cursor/handoff protocol. |
| M07/M13, adapters and cache | Concurrent append/read/close/unmount, failed writeback, mixed bypass/buffered I/O, low-memory reentrancy | Correct lifetimes, coherence and progress; otherwise fix host integration and rerun. |
| M13/M14, full workload qualification | Aged near-full images, multiple seeds, long mixed workload, backup/restore with unknown metadata | Exact content/metadata round-trip and no ownership corruption; reduce failures to permanent fixtures. |

Do not implement a speculative index engine, cache or security format merely
to make these rows executable. Implement the owning feature with its consumer
and these oracles together. Preserve the ability to build missing platform
support rather than treating absent capabilities as permanent limits.

## Normative coverage map

Each row links an invariant family to an executable gate or an explicit
owner decision. Passing one family cannot close an unimplemented family.

| Invariant family | Coverage or required experiment | Owner |
|---|---|---|
| Allocation and shared ownership | `allocation_pressure`, `shared_extents`, `shared_crash`, `alloc_crash`; exact bitmap/reachable ownership | M03/M05 |
| User bytes and bounded resource pressure | `streaming_api`, `reclaim`, `orphans`; mixed I/O, delayed reuse and bounded cleanup; long combined soak additionally required | M03/M13 |
| In-place error semantics | `data_policy::metadata_io_error_after_in_place_data_write_keeps_old_generation_but_not_old_bytes`; metadata integrity does not promise byte rollback after opted-in media writes | M03/M04 |
| Uncertain checkpoint outcome | `faults`: final barrier failure, completed-write error and adoption-read error must block further mutation until remount | M03/M04 |
| Compound replacement rollback | `faults`: dynamically count successful replacement writes/flushes, fail each index, require exact old/new namespace and bytes plus clean ownership | M03/M04 |
| Replay interrupted again | `intent_log::write_and_truncate_replay_is_restartable_after_every_cut`, `intent_replay_orphans` | M04 |
| No-changes and invalid metadata | `mount_modes`, `corruption_corpus`, VFS `no_changes_vfs_remains_readable_and_issues_no_writes_or_flushes` | M05/M07 |
| Open identity and handles | VFS `open_unlinked_file_keeps_identity_until_the_last_handle_closes`, `atomic_replace_preserves_an_open_target_without_exposing_it`; real concurrent cache/VM integration separately required | M07/M13 |
| Exact retained versions | Q4: compare per-file retention and consistent snapshot consumers with space-pressure and reclamation oracles; no arbitrary historical-read claim | M14 |
| Catalog completeness and change handoff | Backfill and reset/overflow experiments above; freeze representations only with a consumer | M09/M10 |
| Salvage, repair and restoration | Q11: corruption-to-extraction corpus, repair accounting and exact backup/restore round-trip | M05/M13/M14 |
| Cache and device durability | Q12: injected errors are host evidence; delayed DMA, failed barriers, sleep/shutdown and native cache coherence need platform qualification | M07/M13/M14 |
| Security preservation | Q5 and `security-model-conformance`; unknown descriptors round-trip through real adapters and transport | M14 |

Run the common gate with `cargo test --workspace --all-features`. The
long-running, native and owner-decision rows cannot be closed by that command.

## Repeated low-space and reclamation gate

`cargo test -p afsplus-check --test allocation_pressure repeated_near_full`
runs 24 cycles on a 512-block volume. Keep a file and its reflink, preallocate
pressure storage to leave bounded headroom, force ENOSPC, attempt a cross-block
COW write, then delete pressure storage and drain reclaim with a fixed step
limit. Each cycle checks exact survivor bytes, checkpoint/free-count stability
on rejected growth, exhaustive ownership and remount. Reclamation must converge
and the next cycle must recover usable capacity.

This exercises repeated pressure on live shared owners. Persistent snapshot
ownership is a separate Q4 qualification under the
[snapshot proposal](../proposals/persistent-snapshot-prototype.md); a reflink
survivor is not a whole-volume snapshot.
