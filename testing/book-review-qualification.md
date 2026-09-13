# Practical File System Design qualification

> **ADRs:** [ADR-067](../adr/ADR-067-epoch1-allocation-state.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** `cargo test -p afsplus-core fragmented_allocation -- --nocapture`; `cargo test -p afsplus-check --test streaming_api` · **Milestones:** M03, M05, M09, M10, M13, M14

The [book review](../docs/33-practical-filesystem-design-review.md) records the
source, chapter coverage and architectural rationale. Results belong in
[milestones](../implementation/milestones.md) and [NOTES](../NOTES.md).

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
