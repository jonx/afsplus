# ADR-097: Share immutable bases across bounded memory overlay branches

Status: Accepted

## Decision

The Stage A overlay is a host test provider with a shared, read-only base and
per-branch block replacements. Ownership prevents writes through the base handle.
Reads consult the branch first, then the base. Base read errors propagate without
installing zero-filled replacements. All accesses validate geometry and exact
buffer length before touching the base or changing branch state.

Use caller-selected limits for live branches and aggregate live replacement
entries across the complete fork family. A fork charges its copied index entries
before publication; branch drop returns those charges. Each retained payload must
have at least one charged live entry. This bounds retained historical payloads
across forks, including diverging rewrites. A replacement write may transiently
allocate one additional block before swapping its entry; admit and document that
scratch overhead per active writer, bounded by the family branch limit. Rewriting an existing entry consumes no additional entry slot.
Admission failure preserves previous branch contents. Report live branch and
entry counts separately from total process memory. Container, allocator and
shared-base memory belong in the later harness's total accounting.

Forking shares the base and immutable replacement payloads. Each branch owns its
index and replaces payloads on write, so subsequent parent/child writes remain
isolated. Cost scales with replaced blocks and index entries; it must not copy
an entire base image. Nested logical partitions use SliceBackend. No merge into
the base, raw-device write, partition discovery or public export is implicit.

This is a memory test provider, with the same process-lifetime storage scope as
MemoryBackend. A flush orders preceding branch writes for the simulation; it
never flushes or modifies the shared base. It provides no persistence across
process exit or real power loss. Fault injection and recorded cut-state replay
remain explicit wrappers/harness operations. A persistent file overlay needs a
separate crash-safe index and payload-publication contract before it can promise
host-file durability.

## Qualification

Run format/mutate/remount/check workflows
with independent branches over one base. Compare every branch's exact file bytes
and every original base block after writes, truncates and namespace changes.
Require missing-block fallback, zero overwrite, boundary and short-buffer
refusal, existing-slot rewrite at capacity, new-slot exhaustion without mutation,
base read errors and repeated forks with isolated payloads.

Measure copied index entries and shared/replaced payload bytes for sparse edits
of a large logical image. If cost scales with base capacity, replace the ownership
or indexing scheme before adopting it. Reject fork or write admission before the aggregate entry/branch limits
can be exceeded. Test capacity recovery after dropping branches and retaining
shared payloads through diverging rewrites. Check record/replay integration with the existing cut-state
oracle; preserve the model's torn/reordered-write coverage and limitations.

Require observable branch isolation, bounded admission and useful
replay integration. Persistent scratch storage remains a distinct requirement
when a workload exceeds the admitted memory profile.

The overlay cut enumerator forks a durable prefix and streams full unflushed
write subsets and the existing sampled tear states. Compare its ordered state
bytes and descriptions with the memory-image oracle, including repeated writes
to one block and multiple block sizes. Branch/entry exhaustion, invalid cut
positions, over-budget tails and consumer errors return incomplete enumeration
explicitly. Partial callbacks never certify completion. Retaining callback
branches consumes the same aggregate family budget.
