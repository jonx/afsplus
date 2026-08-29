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

## What the data supports

1. **Checkpoint-per-operation is not viable as the only durability path**
   for Git/package/database workloads: 9–27 block writes and 3–7 barriers
   per logical operation is a 8–24× write amplification and a ~3× barrier
   multiplier over a log-based design, on the workloads AFS+ names as
   primary targets.
2. **Two separable mechanisms, two problems.** *Group commit* — batching
   many namespace operations into one checkpoint — needs no new on-disk
   structure and fixes throughput for non-fsync bursts (checkout would drop
   to ~0.2 writes and ~0.05 flushes per file at a 64-op window). It does
   nothing for an application that demands durability *now*. The *intent
   log* is specifically the mechanism that makes a forced fsync cost ~1
   write + 1 barrier between checkpoints.
3. **Recommended order:** implement group commit first (pure engine work:
   an operation queue and a checkpoint cadence policy), re-measure, then
   design the intent log for the forced-durability path against these
   numbers, using the auxiliary-log extension point doc 08 already
   reserves. Atomic-replace rename is worth doing alongside: it removes a
   third of the ref-update pattern's cost independently of either mechanism.

The decision itself is a format/architecture call and stays open until the
group-commit measurement exists; this report's role is to establish that
the status quo loses by roughly an order of magnitude on writes and 3× on
barriers, so *some* amortization mechanism is not optional.
