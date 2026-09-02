# fsync under checkpoint COW: measured baseline and intent-log decision input

Measurement report for architecture blocker 2
([`docs/08-transactions-and-journal.md`](../docs/08-transactions-and-journal.md) §8). The harness is
[`crates/afsplus-check/tests/fsync_workloads.rs`](../crates/afsplus-check/tests/fsync_workloads.rs); reproduce with:

```text
cargo test -p afsplus-check --test fsync_workloads --release -- --ignored --nocapture
```

<!-- toc -->

- [Methodology](#methodology)
- [Measured cost per durable operation](#measured-cost-per-durable-operation)
- [The structural floor](#the-structural-floor)
- [Intent-log envelope](#intent-log-envelope)
- [Phase A measured: group commit](#phase-a-measured-group-commit)
- [Phase B measured: the intent log wins its gate](#phase-b-measured-the-intent-log-wins-its-gate)
- [Requalification and coverage boundary](#requalification-and-coverage-boundary)
- [Phase C measured: existing-file durability](#phase-c-measured-existing-file-durability)
- [What the data supports](#what-the-data-supports)

<!-- /toc -->

## Methodology

Every prototype transaction is a full checkpoint commit with its own
durability barriers, so each measured operation is exactly what `fsync`-like
durability costs under checkpoint COW with no batching. Three workloads on a
256 MiB memory-backed volume (4 KiB blocks, 16,384-block regions), measured
through the tracing block backend:

- **git ref update** ×1,000 — create `HEAD.lock` with small content, delete
  the old `HEAD`, rename the lock over it. Three transactions today because
  rename does not yet implement atomic replace; with replace it would be two.
- **checkout small files** ×4,000 — one ~900-byte file per transaction into
  one directory (grows to a multi-level directory tree).
- **durable log append** ×4,000 — 200 bytes appended to one file per
  transaction (full data COW rewrites the touched block).

## Measured cost per durable operation

After the allocation-root cache and roving-allocator changes (reads before
them in parentheses):

| workload             | writes/op | flushes/op | reads/op   | bytes/op |
|----------------------|-----------|------------|------------|----------|
| git ref update       | 27.0      | 7.0        | 50.0 (56)  | ~108 KiB |
| checkout small files | 12.9      | 3.0        | 20.8 (22.8)| ~52 KiB  |
| durable log append   | 9.0       | 3.0        | 19.0 (21)  | ~36 KiB  |

Per-transaction write breakdown (averages): 5–9 COW metadata blocks (object
record, directory path, object-map path, root record), 1 bitmap page, 1
region descriptor, 1 allocation-root node, 1 reclaim-queue root, 1
checkpoint — plus 2 barriers (3 when the transaction carries data).

Wall time in this harness is memory-backend CPU time (~0.7 ms per ref
update) and is not the number that matters: on real storage, **flush count
dominates small-durable-op latency**. At a typical 0.1–5 ms per device cache
flush, 3 flushes per operation is the floor of this design, and the Git
lock-file pattern pays 7.

## The structural floor

No tuning removes this floor. A durable small operation must write at
minimum the checkpoint block plus every root the checkpoint references that
changed — and under publish-per-operation COW, the bitmap page, region
descriptor, allocation-root node, and reclaim root change every time. The
~9-write, 2–3-flush minimum is inherent, which is exactly what doc 08 §8
predicted would need measurement.

## Intent-log envelope

For contrast, the initial namespace-only intent-log projection used one
sequential log-record write plus one barrier per durable operation, with the
full checkpoint amortized over a 64-operation window at today's
per-transaction cost:

| workload             | writes/op est. | flushes/op est. | reduction        |
|----------------------|----------------|-----------------|------------------|
| git ref update       | ~1.1 (×3 ops)  | ~1.05 (×3 ops)  | 8× wr, 2.2× fl   |
| checkout small files | 1.20           | 1.05            | 11× wr, 2.9× fl  |
| durable log append   | 1.14           | 2.05            | 8× wr, 1.5× fl   |

(The append projection includes a data barrier before the record barrier.
The ref-update line stays a three-record sequence until atomic-replace rename
exists; with it, one update ≈ 2 records ≈ 2.3 writes and 2.1 flushes versus
27 and 7 today.)

## Phase A measured: group commit

`Volume::run_batch` now executes a bounded set of namespace operations
(create with content, delete, rename with atomic replace) as ONE
transaction with ONE checkpoint — the ADR-026 bounded atomic batch and the
group-commit mechanism in one primitive. Crash matrices prove the batch is
all-or-nothing (the Git pattern's lock file is never visible in any modeled
crash state). Same harness, same volumes:

| workload               | writes/op | flushes/op | reads/op | vs per-op commit    |
|------------------------|-----------|------------|----------|---------------------|
| checkout batched(64)   | 2.2       | 0.05       | 2.3      | 5.9× wr, 63× fl     |
| ref update batched(2)  | 10.0      | 3.0        | 21.0     | 2.7× wr, 2.3× fl    |

Two findings:

1. **For non-durable bursts, group commit fully closes the gap** — batched
   checkout (2.2 wr/op, 0.05 fl/op) actually beats the intent-log envelope
   (3.2 wr/op est.), because shared tree paths amortize across the batch.
   No intent log is needed for throughput.
2. **The forced-durability floor is now isolated.** A durable ref update as
   one atomic batch costs 10 writes and 3 barriers; an intent log would
   cost ~1.2 writes and ~1 barrier. That ~3× barrier gap on fsync latency
   is the entire remaining case for the intent log — nothing else.

## Phase B measured: the intent log wins its gate

ADR-037's prototype logs each fsync group as one record in a reserved slot
area and replays the valid prefix at mount. Same harness (1,000 durable ref
updates, fsync per update, checkpoint every 64):

| durable ref update path | writes/op | flushes/op | reads/op | wall (mem) |
|-------------------------|-----------|------------|----------|------------|
| original (3 tx)         | 27.0      | 7.0        | 50.0     | 731 ms     |
| group-committed batch   | 10.0      | 3.0        | 21.0     | 286 ms     |
| **intent log, fsynced** | **2.2**   | **1.03**   | **0.4**  | **39 ms**  |

The gate — beat 3 barriers / 10 writes under the same crash matrices — is
passed with a 4.7× write and 2.9× barrier margin (12× / 6.8× against the
original sequence), and the crash matrices hold: one fsync group is one
record, so every modeled state (including a durable record over torn data,
rejected by the content CRC) recovers to exactly the old or the new ref,
with the lock file never visible and stale records inert after any
checkpoint.

Two design points the prototype settled the hard way:

- **Windowed transactions never promote quarantined blocks** (reclaim
  budget zero): logged extents must be FREE in the committed bitmaps so
  replay can claim them deterministically regardless of runtime policy.
- **Cancelling a logged create must not recycle its blocks** — an earlier
  record's content CRC still covers them. Unlogged cancellations scrub the
  pair from the fsync group; logged ones sacrifice the blocks to the
  reclaim queue at commit.

## Requalification and coverage boundary

The same optimized harness was rerun on 2026-09-02 after shared extents and
the Q1 private-in-place experiment landed. The ratios were stable:

| path | writes/op | flushes/op | reads/op | memory-backend wall |
|---|---:|---:|---:|---:|
| intent-log ref update | 2.152 | 1.032 | 0.336 | 25 ms |
| group-committed ref update | 10.0 | 3.0 | 21.0 | 262 ms |
| original ref update | 26.992 | 6.998 | 49.983 | 626 ms |
| checkpointed 200-byte append | 9.037 | 3.0 | 18.962 | 2,575 ms |

The seven intent-log tests and ten shared-extent crash tests also passed in
the optimized build, including every recorded write/flush cut, torn created
content, shared unlink and rename-replacement replay.

That run established the namespace boundary that Phase C subsequently closes.
It remains useful as the before-state: version 2 could publish created-file
content, but not a write or truncate of an already committed file.

## Phase C measured: existing-file durability

Experimental record version 3 adds complete-block COW replacements for writes
and the optional one-block zeroed tail needed by a partial truncate. The
replacement data is written and flushed before its record; the record is then
flushed as the fsync completion point. Replay verifies every content CRC,
claims the exact extents while they are still FREE in the base checkpoint,
and feeds the final layouts through the ordinary COW checkpoint engine.

Optimized qualification on 2026-09-03, with one fsync per operation and a
checkpoint every 64 operations:

| path | operations | writes/op | flushes/op | reads/op | memory-backend wall |
|---|---:|---:|---:|---:|---:|
| logged 200-byte append | 4,000 | 2.200 | 2.032 | 1.396 | 388 ms |
| logged 4 KiB DB hotset | 4,000 | 2.134 | 2.032 | 1.346 | 156 ms |
| checkpointed 200-byte append | 4,000 | 9.037 | 3.000 | 18.962 | 2,590 ms |

The logged append reduces device writes by 4.1× and reads by 13.6× while
removing one of the three barriers per fsync. The database row demonstrates
that repeated overwrites of the same committed file remain bounded rather
than accumulating one checkpoint transaction per fsync. These are
memory-backend structural measurements, not hardware latency claims.

The associated suite now has 16 intent-log tests and 11 shared-extent crash
tests. It includes every modeled write/flush cut around an existing-file
write, monotone recovery across successive write records, write followed by
rename in one group, sparse growth, aligned and partial shrink, a crash at
every write/flush of recovery itself followed by another recovery, and a
logged write that splits a shared extent without changing the clone.

Version 3 is guarded by
`org.aros.afsplus:intent-log-data-updates` (`INCOMPAT` bit 1). Without that
identity, an older version-2 reader could mistake an unknown valid record for
an ignorable torn tail. Namespace-only groups continue to encode version 2;
the numeric v3 wire remains experimental until M14.

## What the data supports

1. **Checkpoint-per-operation is not viable as the only durability path**
   for Git/package workloads: 9–27 block writes and 3–7 barriers per logical
   operation is a 8–24× write amplification and a ~3× barrier multiplier over
   the namespace log path. The Phase C database and append rows now establish
   the corresponding existing-file log cost directly; the separate
   [Q1 bake-off](data-policy-bakeoff.md) compares COW with private in-place
   checkpoint updates.
2. **Two separable mechanisms, two problems — now both measured.** Group
   commit (implemented, measured above) solves burst throughput without
   format change and doubles as the ADR-026 atomic-batch primitive. The
   intent log's remaining value is exclusively the forced-fsync path.
   Namespace updates use about one barrier and 2.2 measured writes per
   operation instead of 3 barriers and 10 writes; existing-file data updates
   use two barriers (data, then record) and about 2.1–2.2 writes instead of the
   three-barrier, 9-write checkpoint path. On storage where a barrier costs
   0.1–5 ms, the difference matters to fsync-per-operation applications and
   nothing for workloads that already batch freely.
3. **Bake-off verdict.** Both mechanisms are now implemented and measured
   under identical workloads and crash matrices. Group commit carries
   bursts (2.2 writes, 0.05 barriers per checkout file); the intent log
   carries forced durability (2.2 writes, 1.03 barriers per fsynced ref
   update — 6.8× fewer barriers than the original sequence). They compose:
   the log is precisely how an fsync becomes cheap between group-committed
   checkpoints. The measured recommendation is to keep both, with the log
   remaining experimental until portable-C parity, real-device qualification
   and the M14 wire review are complete. The public Rust VFS path is now
   qualified by the existing-file log gate.
   [ADR-063](../adr/ADR-063-intent-log-epoch1.md) accepts this layered
   architecture; [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md)
   makes the new replay capability fail closed across implementations.
