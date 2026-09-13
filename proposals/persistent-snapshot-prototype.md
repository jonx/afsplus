# Persistent snapshot prototype

> **ADRs:** [ADR-069](../adr/ADR-069-consistent-snapshots-first.md), [ADR-070](../adr/ADR-070-persistent-snapshot-priority.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [book-review qualification](../testing/book-review-qualification.md) · **Milestones:** M03, M05, M13, M14

Target on acceptance: a snapshot design document, qualification plan and
format/API ADRs; Q4 in [open questions](../implementation/open-questions.md).

The accepted direction is persistent consistent snapshots for the first
backup/scanner consumer. The mechanisms below are candidates for experiments,
not accepted disk semantics or a claim of snapshot support.

## Source constraints

- `TxAllocator::begin` consumes the committed reclaim queue before allocating.
  `ReclaimTx::consume` and `consume_segment` promote runs within the batch
  budget without a snapshot-retention predicate. A saved namespace root alone
  therefore cannot protect old content.
- Reclaim entries carry retirement generation, but not allocation birth.
  They can support conservative generation retention; they cannot precisely
  determine whether a block born after a snapshot ever belonged to that view.
- `write_file_at` allows opted-in non-extending private mappings to overwrite
  in place. Snapshot reachability must override that optimization; a single
  live reference does not establish exclusive ownership across retained views.
- The fixed allocation-root pool has exactly `3N` slots for two selectable
  checkpoints and a new commit. A snapshot cannot retain arbitrary allocator
  roots there without changing the proof and representation.
- The checker accounts for selected roots and quarantined runs. It needs an
  explicit snapshot ownership class and reference rules before older roots
  can be accepted without treating their live blocks as leaks or duplicates.

## S1: Registry and consistent boundary

Candidate: a COW registry of stable snapshot identity, captured metadata
transaction/generation and namespace object-map root. Snapshot reads resolve
namespace/file metadata from that captured root; allocation is governed by
current authoritative allocation state and protected ownership. Do not mount
a saved checkpoint as a writable volume or preserve its old free-space map.

Snapshot creation first resolves pending intent-window work into a checkpoint,
then captures a defined complete view. Publish its registry record through
the common COW engine and acknowledge only after durability. A create with an
uncertain outcome requires remount, discovery and an idempotent caller policy.
Deleting a registry entry likewise uses ordinary checkpoint publication.

Decide where the registry root is negotiated and stored, how snapshot IDs
avoid stale-handle reuse, and what metadata belongs to a view. Explicitly
handle the internal orphan directory: unreachable open files are not ordinary
backup names, but their retained storage still needs correct accounting.
Define whether registry changes themselves appear in a captured view.
No byte layout or feature bit is assigned by this proposal.

## S2: Retention accounting experiment

Compare two candidates with the same workload and immutable view oracle:

| Candidate | Safety argument | Cost and deciding experiment |
|---|---|---|
| Oldest-snapshot generation barrier | A run retired after the oldest retained generation may belong to a snapshot and cannot be promoted. Runs already retired at that boundary are outside its namespace. | Simple conservative baseline; may retain all later unrelated churn and stall FIFO progress. Measure metadata growth and ENOSPC while one old snapshot stays open. |
| Birth/retirement or explicit snapshot ownership accounting | Release only when no retained view intersects a block's lifetime or owns the block. | Requires more ownership metadata and format/checker work. Measure update amplification, bounded deletion and memory against the baseline. |

The first candidate is a useful correctness prototype, not a presumed shipping
policy. A backup taking hours must not cause unexplained space growth from
unrelated temporary files. If that cost is unacceptable, implement more precise
accounting. Do not evade the result by silently expiring promised snapshots.

Retirement generations alone must not be promoted as exact physical snapshot
usage. Report conservative retained capacity distinctly from exact reachable
snapshot data and immediately available space. Blocks shared by multiple
snapshots must not be charged as independently allocated copies.

## S3: In-place writes and all mutation paths

Conservative candidate: force full data COW while any persistent snapshot
exists. Later refine it to proven non-snapshot-owned ranges if measurements
justify the complexity. Cover ordinary writes, intent-window writes, truncation,
preallocation, reflinks, orphan cleanup and direct adapter entry points.

Prove byte immutability for an opted-in file that is captured, overwritten
through several API paths, unlinked and then reclaimed. Existing shared-extent
counts remain counts of the selected live mappings; snapshot ownership must
not be inferred from those counts without a new accepted accounting decision.

## S4: Admission, release and handles

Define a metadata reserve for publishing and deleting registry records under
pressure. Failure to create a snapshot is explicit and publishes no partial
view. Expose retained capacity and exhaustion so users can release views or
add storage deliberately.

Choose whether deletion with active snapshot handles returns busy or removes
the public name while retaining a tombstone until the final reference closes.
If runtime handles disappear after reboot, recovery must distinguish an
explicitly deleted view from a retained persistent view. Test bounded release
and reclamation under interruption before accepting either option.

## Prototype exit criteria

1. Capture a nontrivial tree with hard links, sparse/unwritten data, reflinks
   and opted-in in-place files; retain its exact namespace, metadata and bytes
   in an independent oracle.
2. Create multiple views, mutate the live tree, delete views in different
   orders, remount repeatedly and compare every retained view to its oracle.
3. Enumerate every modeled write/flush cut of create/delete and interrupted
   recovery; accept only complete registered views and safe ownership.
4. Fill the volume while old views survive. Measure admission failure,
   metadata headroom, retained capacity and bounded release/reclaim progress.
5. Cross-read the persistent corpus with the independent C reader or explicitly
   reject the negotiated feature before writes until that support exists.
6. Apply the format/API contribution procedure: ownership invariants,
   compatibility classification, parser corruption cases, repair behavior,
   capability semantics and CPU/RAM/I/O/amplification measurements.

The next decision is the retention prototype strategy in S2. Registry encoding,
handle deletion and admission limits stay explicit Q4 work rather than being
chosen accidentally in an implementation.
