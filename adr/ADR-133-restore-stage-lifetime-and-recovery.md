# ADR-133: Create private restore stages atomically and reclaim them explicitly

Status: Accepted
Amends: ADR-127, ADR-128, ADR-130

## Decision

Create a restore object and its `system.restore.stage` attribute in one core
transaction. The eight-byte root identifier retains ADR-130's encoding. A
standalone file or tree root marks itself; descendants reference that root.
No durable checkpoint may expose an unmarked, unfinished restore object.
Creation requires persistent snapshots before any mutation. Ordinary creation
remains unchanged. Ordinary VFS access rejects an active or malformed marker;
directory enumeration skips such an entry while preserving unrelated siblings.
Volumes without persistent snapshots need no restore-marker lookups.

Retained stages have an explicit maintenance lifetime. Authorized restore
maintenance scans a caller-selected live directory subtree, with bounded entry,
work and depth budgets per call. It never runs an exhaustive scan at mount.
The resumable cursor binds the volume, scope and checkpoint generation; a stale
cursor requires a safe restart. Persist cursor files outside the repaired volume
so saving a cursor cannot invalidate it. Concurrent modifications can require a
restart; maintenance of a quiescent subtree is the supported completion path.

Only an exact active root marker and matching descendant markers permit reclaim.
Validate the parent entry and expected object identifier before each mutation.
Malformed markers, unmarked children or inconsistent edges block that stage and
are reported, never guessed away. Continue independent siblings. Published
objects with inactive derived markers are never reclaim candidates. The caller
can select a deeper live scope without increasing the traversal depth bound.

Reclaim file data in bounded extent-record steps using existing snapshot-aware
release accounting, then remove empty objects from leaves to root. This does
not require the orphan feature. Progress remains durable across interruption;
losing a cursor permits another scan. Work budgets bound entries, objects and
extent records, not every physical block touched by a metadata transaction.
Existing attribute and metadata admission ceilings continue to apply. An end
of scan with blocked stages reports partial completion, not success.

If public publication may have committed but its durability outcome is unknown,
return an explicit publication-uncertain result. Do not report an ordinary
prepublication failure or advise blindly retrying. Failures known to precede
publication retain their original error. Publication within a private tree does
not expose it while its root remains marked. A caller must inspect the final
destination after remount before deciding whether to retry an uncertain result.

## Qualification

Record real writes and flushes and enumerate every operation-prefix crash cut,
all modeled full-write subsets of the unflushed tail and representative tears.
Check empty stages as well as populated ones, including direct-ID access and
mutation. Final publication is absent or complete with historical bytes and
metadata. Retain the old implementation as a failing control. Exercise bounded
reclaim, interruption, remount, stale cursors, malformed and unrelated objects,
snapshot data retention and publication uncertainty. Report actual I/O costs
before removing redundant barriers; crash-state evidence must remain passing.
