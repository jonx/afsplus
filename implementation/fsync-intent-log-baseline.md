# fsync under checkpoint COW: measured baseline and intent-log decision input

Status: measurement report for architecture blocker 2
(`docs/08-transactions-and-journal.md` §8). The harness is
`crates/afsplus-check/tests/fsync_workloads.rs`; reproduce with:

```text
cargo test -p afsplus-check --test fsync_workloads --release -- --ignored --nocapture
```

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

For contrast, a minimal intent log — one sequential log-record write plus
one barrier per durable operation, with the full checkpoint amortized over a
64-operation window at today's per-transaction cost:

| workload             | writes/op est. | flushes/op est. | reduction        |
|----------------------|----------------|-----------------|------------------|
| git ref update       | ~1.1 (×3 ops)  | ~1.05 (×3 ops)  | 8× wr, 2.2× fl   |
| checkout small files | 1.20           | 1.05            | 11× wr, 2.9× fl  |
| durable log append   | 1.14           | 1.05            | 8× wr, 2.9× fl   |

(The ref-update line stays a three-record sequence until atomic-replace
rename exists; with it, one update ≈ 2 records ≈ 2.3 writes and 2.1 flushes
versus 27 and 7 today.)

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

## What the data supports

1. **Checkpoint-per-operation is not viable as the only durability path**
   for Git/package/database workloads: 9–27 block writes and 3–7 barriers
   per logical operation is a 8–24× write amplification and a ~3× barrier
   multiplier over a log-based design, on the workloads AFS+ names as
   primary targets.
2. **Two separable mechanisms, two problems — now both measured.** Group
   commit (implemented, measured above) solves burst throughput without
   format change and doubles as the ADR-026 atomic-batch primitive. The
   intent log's remaining value is exclusively the forced-fsync path:
   3 barriers + 10 writes per durable update versus ~1 barrier + ~1.2
   writes. On storage where a barrier costs 0.1–5 ms, that is roughly a
   3× fsync-latency difference for fsync-per-operation applications (Git,
   databases), and nothing for everyone else.
3. **The remaining decision** is therefore narrow: is ~3× on forced-fsync
   latency worth a new on-disk structure (log area, record format, replay
   in recovery, its own crash matrices)? That is a product call about how
   central fsync-heavy workloads are. If yes, the log is designed against
   the auxiliary-log extension point doc 08 reserves, and its bake-off
   gate is: beat 3 flushes and 10 writes per durable ref update under the
   same crash matrices. If no, group commit plus the documented 3-barrier
   fsync floor is a defensible v1 stance, and the log stays a negotiable
   future feature.
