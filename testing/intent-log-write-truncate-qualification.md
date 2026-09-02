# Intent-log existing-file update qualification

> **ADRs:** [ADR-063](../adr/ADR-063-intent-log-epoch1.md),
> [ADR-064](../adr/ADR-064-intent-log-data-update-compatibility.md) ·
> **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** `crates/afsplus-check/tests/intent_log.rs`,
> `crates/afsplus-check/tests/shared_crash.rs`,
> `crates/afsplus-check/tests/fsync_workloads.rs` · **Milestones:** M04, M14

This gate qualifies experimental intent-log version 3 for writes and
truncates of already committed files. It does not freeze the wire format or
claim that the portable VFS and C implementations expose the path yet.

## Durability oracle

An existing-file update in an open window follows this order:

1. construct complete replacement blocks from the current logical view;
2. allocate fresh physical blocks and write the replacement data;
3. flush that data before writing a record that names it;
4. write and flush one checksummed record for the complete fsync group;
5. later materialize all logged groups through one ordinary COW checkpoint.

Before fsync completes, a crash may recover the old file. After it completes,
mount must replay the record and recover the new file. No state may expose a
mixture of old and new logical blocks. Successive completed groups may recover
only a monotone prefix of their record sequence.

The data extents named by every active record must be allocatable but FREE in
the base checkpoint bitmap, pairwise disjoint across the valid record prefix,
within the resulting logical file size and protected by a CRC over every
complete replacement block. A partial shrinking truncate may name exactly one
zero-tailed block; sparse growth, aligned shrink and a shrink whose retained
tail is a hole carry no data.

## Compatibility oracle

Any group containing `Write` or `Truncate` uses record version 3 and requires
`org.aros.afsplus:intent-log-data-updates` (`INCOMPAT` bit 1), which itself
requires the base intent-log bit. A version-3 record without bit 1 is
corruption. A volume carrying only the historical bit 0 remains usable for
namespace-only version-2 records, while its existing-file window API returns
`FeatureDisabled`.

This fail-closed rule is essential: the historical scanner treats an unknown
record version as an invalid tail, which would otherwise let an older writer
silently discard an acknowledged fsync.

## Reproduction

Run codec, intent-log and shared-reference correctness gates:

```text
cargo test -p afsplus-format --test roundtrip
cargo test -p afsplus-check --test intent_log
cargo test -p afsplus-check --test shared_crash
```

Run the optimized 4,000-operation measurement:

```text
cargo test -p afsplus-check --test fsync_workloads --release \
  fsync_workload_qualification -- --ignored --nocapture
```

The 2026-09-03 run passed 29 codec tests, 16 intent-log tests and 11
shared-crash tests. Its relevant rows were:

| workload | operations | writes/op | flushes/op | reads/op |
|---|---:|---:|---:|---:|
| logged 200-byte append, checkpoint/64 | 4,000 | 2.200 | 2.032 | 1.396 |
| logged 4 KiB DB hotset, checkpoint/64 | 4,000 | 2.134 | 2.032 | 1.346 |
| checkpointed 200-byte append | 4,000 | 9.037 | 3.000 | 18.962 |

These are deterministic memory-backend structural counts. Hardware latency,
cache persistence and throughput remain separate real-device gates.

## Crash matrix

The executable cases prove:

- every cut around a logged existing-file write yields the exact old or new
  content and a checker-clean image;
- a write and partial truncate replay in order, preserving timestamps and
  content generation semantics;
- a write followed by rename is one all-or-nothing fsync group;
- restarting after every modeled write/flush of recovery itself converges on
  the same final checkpoint;
- successive writes recover only record-prefix contents;
- sparse growth, aligned shrink and partial shrink choose the correct
  data-free or one-tail-block representation;
- a logged write to one reflink owner splits shared-reference accounting while
  the other owner retains its original bytes, including crashes during
  recovery;
- bit-0-only volumes retain namespace replay but reject data updates, and a
  version-3 record with no bit 1 fails closed in both mount and checker.

## Remaining boundary

The Rust core APIs and recovery path are qualified. The current log record is
bounded to one block and at most 16 physical extents per data operation.
Deleting or replacing a file already modified inside the same open window is
an explicit `PrototypeLimit` until a compaction rule is specified. Public VFS
fsync wiring, portable-C parity, real-storage flush testing and final numeric
wire allocation remain M14 work.
