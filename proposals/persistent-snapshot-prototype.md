# Persistent snapshot prototype

> **ADRs:** [ADR-069](../adr/ADR-069-consistent-snapshots-first.md), [ADR-070](../adr/ADR-070-persistent-snapshot-priority.md) · **Spec:** [invariants](../spec/invariants.md) ·
> **Tests:** [book-review qualification](../testing/book-review-qualification.md) · **Milestones:** M03, M05, M13, M14

Target on acceptance: a snapshot design document, qualification plan and
format/API ADRs; Q4 in [open questions](../implementation/open-questions.md).

The accepted direction is persistent consistent snapshots for the first
backup/scanner consumer. The mechanisms below are candidates for experiments,
not accepted disk semantics or a claim of snapshot support.

<!-- toc -->

- [Source constraints](#source-constraints)
- [S1: Registry and consistent boundary](#s1-registry-and-consistent-boundary)
- [S2: Retention accounting experiment](#s2-retention-accounting-experiment)
  - [Executable comparison and integration gate](#executable-comparison-and-integration-gate)
- [S3: In-place writes and all mutation paths](#s3-in-place-writes-and-all-mutation-paths)
- [S4: Admission, release and handles](#s4-admission-release-and-handles)
- [S5: Integrated accounting candidate for review](#s5-integrated-accounting-candidate-for-review)
- [Prototype exit criteria](#prototype-exit-criteria)

<!-- /toc -->

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

Start with the conservative oldest-snapshot barrier as the measurement baseline.
Compare precise accounting against the same workload and immutable view oracle
if the baseline fails the agreed resource budgets:

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

### Executable comparison and integration gate

Use the [accounting model gate](../testing/book-review-qualification.md#snapshot-accounting-model)
for lifetime boundaries, unrelated churn and bounded scan progress. Integrate
birth generations or explicit ownership with last-live-reference retirement,
including reflinks. A retirement record alone cannot recover allocation birth.
Precise predicates also need traversal past protected entries; retaining the
sealed FIFO head would preserve the churn failure even with exact ownership.

Before selection, propose storage for lifetime information and the durable
scan cursor, bound snapshot lookup work, and measure queue rewrite I/O.
Separate snapshot namespace reachability from allocator and reclaim metadata
needed by selectable checkpoints. Include crash-safe release, emergency
metadata headroom and independent checker ownership in the integrated gate.

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

## S5: Integrated accounting candidate for review

Decision requested: prototype a physical-run lifetime ledger and bounded
persistent traversal, with full COW while snapshots exist. This chooses the
next integrated experiment. Shipping encoding and resource budgets require
its evidence and a format ADR.

The ledger maps physical starts to length, allocation birth, and optional
last-live-reference retirement generation. Adjacent runs merge only when all
lifetime fields match. Splitting a reflink run preserves its original birth;
creating another live reference never resets birth. Retirement occurs when
the last live mapping disappears. The shared-extent tree keeps its live-count
meaning. A retained generation S owns a retired run exactly when
`birth <= S < retirement`, under the continuous-live-lifetime assumption.
Importing a mapping from an old snapshot requires a separate lifetime rule
before exposing snapshot-to-live reflinks or rollback.

Track namespace object-map nodes, object records, directory/extent nodes and
file data. Classify allocation-root pages, bitmap/descriptor slots, reclaim
queue nodes, snapshot-registry nodes and ledger nodes as housekeeping. Their
lifetime follows selectable-checkpoint protection. They never recursively
insert themselves into the snapshot ledger. Require an exhaustive checker to
verify the classification and both ownership domains, including orphan data.

Retired snapshot-domain runs enter the ledger's retained state. A durable
cursor scans a bounded number of ledger records per transaction and transfers
eligible runs to ordinary checkpoint quarantine. It advances past protected
records, wraps explicitly, and preserves progress across remount. Transfer
and ledger removal commit atomically; ordinary quarantine supplies the existing
selectable-checkpoint delay before physical reuse. Test exact bitmap accounting
so transferred runs cannot be charged, freed or queued twice. New allocations
behind the cursor are visited on the next wrap. Final snapshot release records
its registry change atomically; reclamation proceeds incrementally afterwards.

Candidate tree values use independent integer fields and explicit reserved
bytes. Assign tree identities and checkpoint root fields in the format ADR;
never reuse the checkpoint's unresolved reserved flags. The registry needs a
monotonic next-ID counter, captured generation, committed transaction ID and
object-map root. Exhaustion must fail explicitly, without wrapping IDs.
A snapshot-aware reader resolves only captured namespace state. Mount reads
bounded roots and the cursor; exhaustive ledger scans belong to maintenance.

Compatibility candidate: a negotiated incompatible feature because old tools
cannot validate the extended checkpoint and ownership semantics. Keep legacy
feature-absent images readable; independently test old readers rejecting new
images before mutation. Require Rust/C corpus agreement on root references,
record ordering, arithmetic overflow, reserved bytes and cursor recovery.

Admission candidate: reject new snapshots explicitly when their configured
count or metadata-reserve budget would be exceeded. Preserve registered views
until explicit deletion. Measure worst-case ledger/registry tree split and
ordinary quarantine publication costs to size emergency reserves. Report live,
retained, quarantine and immediately available capacity separately. A physical
block shared across snapshots consumes capacity once.

Handle candidate: return busy on deletion while local snapshot readers hold
handles. This keeps the first persistence contract free of deferred-deletion
tombstones. Closing handles permits explicit deletion; a reboot releases
runtime handles while preserving registered views. A later unlink-like policy
requires a separate decision and crash-safe tombstone proof.

Integration measurements include each mutation path, last-reference transitions,
old-view temporary churn, maximum configured snapshot count, queue wrap,
create/delete admission at ENOSPC, ledger split/rebalance, and every publication
cut. Count ledger/registry/quarantine I/O, reclaimed blocks per step, peak RAM,
read amplification and emergency headroom. Compare against the same image and
workload with snapshots disabled. Passing the isolated model supplies only the
initial lifetime and traversal oracle.

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

The experiment starts with the S2 conservative baseline. Registry encoding,
handle deletion and admission limits stay explicit Q4 work rather than being
chosen accidentally in an implementation.
