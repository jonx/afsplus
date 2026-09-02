# Data-update policy qualification

> **ADRs:** none while Q1 remains open · **Spec:** none ·
> **Tests:** `crates/afsplus-check/tests/data_policy.rs` · **Milestones:** M03, M14

This plan compares full data copy-on-write with the conservative private
in-place prototype without changing the disk format. It is the executable
decision input for open question Q1.

## Reproduction

Run the correctness and crash-contract smoke suite in the normal workspace
gate:

```text
cargo test -p afsplus-check --test data_policy
```

Run the 1,000-operation workload matrix using the optimized build:

```text
cargo test -p afsplus-check --test data_policy --release \
  data_policy_workload_qualification -- --ignored --nocapture
```

The backend is a 512 MiB memory image with 4 KiB blocks and 64 MiB allocation
regions. Tracing starts after image creation and workload setup so the table
contains only the operation phase. A deterministic linear-congruential
sequence chooses random blocks.

## Workloads and metrics

Each policy runs on a fresh image:

- `random-4K`: 1,000 full-block rewrites across a 16 MiB file;
- `db-hotset-4K`: 1,000 rewrites across a fixed 32-page working set;
- `append-4K`: 1,000 extending writes;
- `reflink-first-write`: one first write to each of 1,000 shared blocks.

The harness reports traced reads, writes and flushes, blocks overwritten in
place, transaction allocations and retirements, final extent count, peak
resident allocator bitmap payload, and memory-backend wall time. Each final
image passes the exhaustive checker. Wall time is a deterministic CPU-heavy
comparison of the algorithms, not a hardware latency or throughput claim;
process RSS and CPU user/system time belong in the later real-storage gate.

## Crash oracles

The default full-COW path retains its existing exact-state oracle: every
modeled power cut recovers either the old generation with the complete old
bytes or the new generation with the complete new bytes.

The private-in-place oracle deliberately differs. Metadata and allocation
state must remain checker-clean, and a published new generation must contain
the complete new bytes. When the older checkpoint wins, bytes outside the
requested range remain unchanged, but bytes inside it may be old, new, or a
representative torn mixture. The model enumerates all full-write subsets of
the unflushed tail plus its documented representative block tears.

A deterministic injected-error case also fails the first metadata write after
the in-place data barrier. The operation returns an error and the generation
does not advance, but the old generation observes the new data bytes. This is
part of the policy contract, not a retry guarantee. A three-block unaligned
write separately proves that every touched private mapping is reused.

Shared, unwritten, sparse, or extending writes are ineligible for in-place
overwrite and must continue to satisfy the full-COW exact-state oracle.

## Acceptance boundary

The experiment can justify an explicit hybrid policy only if it materially
reduces write amplification, allocations/retirements and fragmentation on
random and database workloads without changing the shared-range rules. It
cannot justify automatic policy selection, a universal old-generation byte
guarantee, or a hardware-performance claim.
