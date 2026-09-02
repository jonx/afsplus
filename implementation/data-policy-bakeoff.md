# Q1 data-policy bake-off

Measurement report for architecture blocker Q1. The reproducible method and
semantic oracles are in
[`testing/data-policy-qualification.md`](../testing/data-policy-qualification.md);
the implementation is a runtime-only experiment and changes no wire format.

## Prototype boundary

`DataUpdatePolicy::FullCow` remains the default on every mount.
`InPlacePrivate` overwrites the committed physical block only when the whole
operation is a non-extending write and every touched block belongs to a
materialized extent whose flags are clear. Any hole, unwritten extent, shared
marker, extension, or uncertainty sends the complete operation through the
existing COW path. Remounting resets the runtime choice to full COW.

This intentionally isolates the architecture tradeoff. It contains no
per-file on-disk policy bit and does not stabilize a public policy API.

## Optimized qualification result

Apple Silicon host, memory block backend, 4 KiB blocks, 1,000 transactions per
row, 2026-09-02:

| workload | policy | writes | reads | flushes | in-place blocks | allocated | retired | final extents | wall |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|
| random 4K / 4,096 blocks | full COW | 26,972 | 56,769 | 3,000 | 0 | 24,870 | 21,936 | 1,767 | 73,589 ms |
| random 4K / 4,096 blocks | private in-place | 8,000 | 16,000 | 3,000 | 1,000 | 3,000 | 2,000 | 1 | 179 ms |
| database hot set / 32 blocks | full COW | 9,000 | 18,997 | 3,000 | 0 | 5,000 | 3,999 | 31 | 1,106 ms |
| database hot set / 32 blocks | private in-place | 8,000 | 16,000 | 3,000 | 1,000 | 3,000 | 2,000 | 1 | 165 ms |
| append 4K | full COW | 8,999 | 17,994 | 3,000 | 0 | 4,999 | 2,998 | 11 | 518 ms |
| append 4K | private in-place | 8,999 | 17,994 | 3,000 | 0 | 4,999 | 2,998 | 11 | 516 ms |
| reflink first write | full COW | 10,000 | 22,000 | 3,000 | 0 | 6,000 | 4,000 | 10 | 586 ms |
| reflink first write | private in-place | 10,000 | 22,000 | 3,000 | 0 | 6,000 | 4,000 | 10 | 569 ms |

For the large random file, the private path requests 3.37 times fewer device
writes, 8.29 times fewer allocations and 10.97 times fewer retirements, while
ending with one extent rather than 1,767. Its 412-times lower memory-backend
wall time is chiefly the avoided growth and repeated mutation of the extent
tree; it is algorithmic evidence, not a prediction of SSD throughput. The
32-page database hot set still reduces total writes by 11%, allocations by
40%, retirements by 50%, and fragmentation from 31 extents to one.

Append and first writes to shared blocks are deliberately identical across
policies. That negative result confirms the eligibility boundary rather than
claiming a universal speedup. Both modes retain the same three flushes per
data transaction, so in-place overwrite does not solve forced-durability
latency. [ADR-063](../adr/ADR-063-intent-log-epoch1.md) addresses that separate
mechanism.

## Crash and historical-generation cost

The full-COW matrix recovers exact old or exact new contents. The in-place
matrix keeps both checkpoint metadata graphs valid but demonstrates an older
generation containing old, new, or torn bytes within the overwritten range.
This is the direct cost of serving hot-file workloads without data COW: an
exact historical content handle cannot promise byte stability for a file that
opted into the weaker policy.

The checker remains clean in every modeled state because allocation and
metadata ownership do not change when a private physical block is overwritten.
That is structural consistency, not proof of old-data atomicity.

The same boundary holds for reported device errors: when the data write and
its barrier succeed but a later metadata write fails, `write_file_at` returns
an error and does not advance the generation, yet the old checkpoint reads the
new bytes. The injected-error regression pins this behavior so callers cannot
mistake a failed in-place transaction for byte rollback.

## Decision supported by the evidence

The measured benefit supports the explicit hybrid accepted by
[ADR-062](../adr/ADR-062-explicit-hybrid-data-updates.md): full COW by default,
with per-file opt-in to private in-place updates and an explicit loss of
historical byte stability. Automatic selection is not supported by the
evidence. Shared or uncertain ranges remain mandatory COW.
