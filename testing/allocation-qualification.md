# Allocation-State Qualification

> **ADRs:** [ADR-021](../adr/ADR-021-deferred-reclamation.md),
> [ADR-035](../adr/ADR-035-allocation-root-reserved-pool.md),
> [ADR-036](../adr/ADR-036-reclaim-queue.md) · **Spec:**
> [invariants](../spec/invariants.md) · **Tests:**
> [crash testing](crash-testing.md) · **Milestones:** M03, M14

This plan supplies the deciding evidence for open question Q3: whether the
triple-slot bitmap and descriptor representation plus the reserved `3N`
allocation-root pool can become the epoch-1 allocation encoding. It combines
low-space forward progress, exact ENOSPC behavior, large-region amplification,
multi-node allocation-root pressure and the existing crash/reclamation
matrices.

## Executable gates

The normal correctness and power-cut gate is:

```text
cargo test -p afsplus-check --test allocation_pressure
```

It proves that an allocation returning `NoSpace` performs no writes or
barriers and publishes no generation, that delete plus bounded reclaim can
recover deliberately retained headroom, that a near-full delete recovers to
exactly its pre- or post-transaction state at every modeled power cut, and
that one transaction can dirty at least 144 regions across a multi-node
allocation root with bounded allocator memory. The VFS gate additionally
fills a 160-region image, unlinks a highly fragmented final-link file through
the bounded orphan transition, and drains it with an extent budget of eight
while the caller's general reclaim budget is deliberately only one block.

Run both optimized evidence workloads with:

```text
cargo test -p afsplus-check --test allocation_pressure --release \
  -- --ignored --nocapture
```

Each output row is one JSON object with `schema_version: 1`. For local CPU
user/system accounting, wrap the optimized command with `/usr/bin/time -lp`
on macOS or `/usr/bin/time -v` on systems providing GNU time. In-process
elapsed time is a memory-backend algorithm measurement, not a storage-device
latency claim.

## Evidence workloads

`one-gib-region-near-full` reserves almost every free block in the maximum
1-GiB allocation region. It reports elapsed time, logical reads/writes and
bytes, barriers, allocation-root/descriptor/bitmap write counts, allocator
peak bytes, and total and metadata bytes per reserved block in millionths of
a byte. The final exhaustive checker is mandatory.

`low-space-delete-progress` sweeps requested headroom on two geometries:

- one 512-block region, isolating the minimum transaction machinery;
- 145 16-block regions, crossing the 144-record allocation-tree leaf bound
  while producing a highly fragmented preallocated file and a segmented
  reclaim update.

Every row reports whether the fill committed, raw free blocks after that
commit, the runtime emergency floor, normally available blocks, whether unlink
committed, and the successful unlink's allocation, retirement, metadata-write
and reclaim-structure counts. Growth must either commit without crossing the
floor or fail before its first media write. This is a diagnostic freeze
workload: a green Rust test means the measurement completed and every
committed image was structurally valid, not that its Q3 acceptance condition
was met.

The current format-neutral policy is
`clamp(ceil(total_blocks / 32), 8, 64)` blocks on volumes of at least 64
blocks; smaller test-only geometries retain a zero floor. The maximum is 256
KiB at 4 KiB blocks and is not a dedicated disk area. Destructive/recovery
transactions may consume it; ordinary growth cannot.

## Q3 acceptance boundary

The encoding is eligible for freeze only when all of these are true:

- failed growth at ENOSPC is media-I/O-free and leaves the mounted state
  unchanged;
- normally advertised allocatable space excludes enough emergency metadata
  headroom for bounded unlink, orphan cleanup, reclaim and repair progress;
- raw free space and normally available space are reported separately;
- low-space transactions and every modeled crash state pass the exhaustive
  checker;
- the maximum-region and multi-node cases retain bounded allocator memory and
  have recorded metadata/write amplification;
- the G1/G2/G3/G4 quarantine matrix, bitmap-page boundary cases, 145-region
  allocation-root crash matrix and sparse 1-TiB mount qualification remain
  green.

The observed 3- and 5-block delete thresholds are not format constants. The
chosen 8-to-64-block rule is deliberately conservative relative to those
fixtures, and the fragmented VFS workload proves that visible unlink no
longer scales with the file layout. The rule remains runtime policy: it does
not turn into a fixed metadata partition or alter bitmap/checkpoint encoding.

## Repeated retained-view pressure

Run the hosted persistent-snapshot fixture:

```text
cargo test -p afsplus-core repeated_near_full_cycles -- --nocapture
```

A 512-block memory image with 4096-byte blocks carries a long-lived snapshot,
shared source/writer files and a rotating snapshot. Each of 24 cycles fills
ordinary capacity to a 32-block allowance above the emergency floor, refuses an
oversized reservation without writes, attempts a cross-block COW write, deletes
the rotating snapshot and filler, and recovers more than 384 available blocks
within 512 maintenance calls. Open snapshot handles must make deletion busy.

Run lifetime scans with budgets of one and eight records, an edit budget of 512,
a view budget of four and a reclaim promotion budget of 64 blocks. Each
maintenance call must respect its record budget. Compare live and historical
bytes before and after maintenance and remount. Exhaustively validate both
selectable checkpoints and their registered snapshots after every cycle;
absence of the deleted registry identity is checked after reboot simulation.

The output records scan/promotion/call totals, admitted writes, minimum free
space, and maintenance-only device reads/writes/bytes/barriers and elapsed
nanoseconds. The maintenance bitmap peak excludes descriptors, trees, other
buffers and the memory backend; it is not process peak RAM. Timed and traced
maintenance excludes fill, mutation, byte oracles, exhaustive verification and
mount. Debug memory-backend timing is algorithm evidence, not device latency or
native resource qualification. Scan budget changes may produce different
quarantine histories and promotion totals; both profiles must meet the same
capacity and exact-byte gates.

Aged multi-region workloads, sustained real consumers, full memory accounting
and authorized hardware lifecycle tests belong to their separate gates. This
fixture complements the per-boundary crash matrices; it does not enumerate
power cuts inside every operation of every pressure cycle.
