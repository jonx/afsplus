# Data-update policy qualification

> **ADRs:** [ADR-062](../adr/ADR-062-explicit-hybrid-data-updates.md) · [ADR-065](../adr/ADR-065-persistent-data-update-policy.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** `crates/afsplus-check/tests/data_policy.rs`, `crates/afsplus-check/tests/data_policy_persistence.rs` · **Milestones:** M03, M14

This plan compares full data copy-on-write with the conservative private
in-place prototype and qualifies the persistent per-file opt-in that
ADR-065 encodes. `data_policy.rs` exercises the runtime qualification
switch; `data_policy_persistence.rs` proves the ADR-065 contract — the
opt-in survives remounts and every layout rewrite, drives the in-place path
exactly where the ADR-062 eligibility rules pass, shared blocks of a
flagged file still take COW, a flagged record on a volume without the
`COMPAT` feature is corruption for both the read path and the checker, and
the power-cut contract holds through the persistent flag rather than the
switch.

<!-- toc -->

- [Reproduction](#reproduction)
- [Workloads and metrics](#workloads-and-metrics)
- [Crash oracles](#crash-oracles)
- [Acceptance boundary](#acceptance-boundary)
- [Private unwritten reservation initialization](#private-unwritten-reservation-initialization)
- [Bounded reservation edits](#bounded-reservation-edits)
- [Bounded writes](#bounded-writes)
- [Sparse growth](#sparse-growth)

<!-- /toc -->

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


## Private unwritten reservation initialization

[ADR-079](../adr/ADR-079-initialize-private-unwritten-reservations.md) requires
`cargo test -p afsplus-core reservation_write --all-features -- --nocapture`.
Compare actual physical addresses before and after a mixed written/unwritten
write. Private unwritten blocks must keep their address; written, shared and
stale-marked portions must preserve their required COW behavior. Verify exact
bytes, partial-block zeros, captured snapshots, remount and exhaustive ownership
for both checkpoints. A valid-CRC false-private marker over shared references
must be refused before any data write or flush.

Use a low-space image with less free data capacity than the reserved write
requires. Require successful initialization using the existing reservation,
exact snapshot zeros, and separate initialized-block, metadata, byte and flush
counters. Metadata headroom is a prerequisite; this test does not promise
success with no publication space.

Enumerate every modeled write-publication cut and require exact old zeros or
complete new bytes, with historical zeros and both-slot ownership unchanged.
Inject a completed data write followed by an error and failures at each data,
metadata and checkpoint barrier. Before publication, old logical bytes must
survive despite changed unwritten physical payload. Uncertain checkpoint
publication must reject subsequent mutation until reconciliation/remount.

Run [check-reservation-portability.sh](../tools/check-reservation-portability.sh)
for independent portable C reads of original, initialized and older-checkpoint
fallback images. The [fixture generator](../crates/afsplus-check/src/bin/afsplus-reservation-fixture.rs)
uses the baseline format without snapshot-registry negotiation; the fallback
image invalidates the newer checkpoint after physical initialization. Require
exact logical zeros from the older unwritten mapping, and exact initialized
bytes plus zero tails from the newer written mapping. This does not qualify a
C snapshot-registry consumer.

The [C probe](../portable/c/tests/reservation_probe.c) uses 6 KiB scratch for
directory traversal, 4 KiB for object lookup/file reads and a 257-byte data
buffer. It must reject insufficient lookup scratch before I/O. Strict C99 and
ASan/UBSan runs must pass all three images, and an initialized-image-as-zeros
negative control must fail with a byte mismatch. Report callback reads. Caller
scratch sizes exclude stack frames, stdio and host process memory.

An executable `AFSPLUS_M68K_CC` or the script's documented default toolchain
path enables compile-only M68000 checks and a maximum static reader frame
report. Missing compiler paths produce an explicit skip. Compilation is separate
from emulator/hardware execution and total stack-depth/RAM qualification.
Older-target resource profiles, bounded fragmented-layout traversal and sustained
near-full reservation workloads require separate evidence.

## Bounded reservation edits

Run `cargo test -p afsplus-core bounded_reservation --all-features -- --nocapture`.
A one-block edit in a file with 600 fragmented extents must read fewer than
150 device blocks and write fewer than 20 metadata blocks. Check live and
captured allocation accounting with full reachable-state verification.
Exercise empty, before-first, interior, after-last and already-reserved windows.
Block, input-record and result-record refusals must issue zero writes; retrying
with sufficient limits must preserve the original mappings and succeed.

For a multi-node tree, enumerate crash states around every publication cut.
Require the complete old or new live layout, exact captured allocation ranges,
and both selectable checkpoint graphs. Report the old/new outcome counts.
The model limitations in [crash-testing](crash-testing.md) apply.

The record budget bounds local extent vectors. Peak process RAM, allocator
working sets, ordinary writes/truncation, and constrained native execution need
separate measurements before a complete memory profile can be qualified.

## Bounded writes

Run `cargo test -p afsplus-core bounded_writes --all-features -- --nocapture`.
Exercise a 130-record fragmented file with mixed reservation/hole writes,
subsequent written-data replacement and an end-of-file write. Each admitted
small write must use fewer than 180 device reads; compare full live bytes,
captured zeros and remounted bytes. Block and record refusals must issue zero
writes and preserve the object record. Enumerate publication crash cuts for a
mixed reservation/hole write and require exact old or new live bytes with the
captured view unchanged. Full checker validation covers both checkpoint slots.

Also verify shared-peer isolation under requested private-in-place policy, and
an eligible private tree on a volume without snapshot support. The eligible
case must retain physical mappings and report the exact in-place counter.
Exercise empty/direct promotion through the bounded entry point.

## Sparse growth

Run `cargo test -p afsplus-core sparse_growth --all-features -- --nocapture`.
Grow a 300-record fragmented file to `u64::MAX`, requiring fewer than 100
device reads and the identical extent root, block count and allocated byte
count. Verify logical zeros in the final address block before and after
remount, and preserve the snapshot's original bytes. The former overflowing
block-end expression must fail this case as a negative control.

Enumerate growth publication cuts with a retained snapshot and unwritten
reservations. Require the exact old or new object, corresponding full bytes,
unchanged extent root and captured allocation. Full graph checking covers
both selectable checkpoints. This gate covers growth; bounded shrinking and
total allocator/retention RAM have separate resource requirements.
